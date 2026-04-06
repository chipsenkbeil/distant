//! Mount plugin implementations for each backend.
//!
//! Each plugin wraps the backend-specific mount logic behind the
//! [`MountPlugin`](distant_core::plugin::MountPlugin) trait, allowing the
//! manager to orchestrate mounts without knowing backend details.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Mutex;

use distant_core::Channel;
use distant_core::plugin::{MountHandle as MountHandleTrait, MountPlugin};
use distant_core::protocol::MountConfig;

use crate::core::MountHandle as ConcreteMountHandle;

/// Wraps the concrete [`MountHandle`](crate::core::handle::MountHandle) to
/// implement the [`MountHandle`](distant_core::plugin::MountHandle) trait.
///
/// Uses a `Mutex` around the inner handle to satisfy `Sync` (required by
/// the trait for `RwLock` compatibility in the manager).
struct MountHandleWrapper {
    inner: Mutex<Option<ConcreteMountHandle>>,
    mount_point: String,
}

impl MountHandleTrait for MountHandleWrapper {
    fn unmount(&mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>> {
        Box::pin(async {
            let handle = self.inner.lock().unwrap().take();
            if let Some(handle) = handle {
                handle.unmount().await
            } else {
                Ok(())
            }
        })
    }

    fn mount_point(&self) -> &str {
        &self.mount_point
    }
}

// NFS

/// Mount plugin for the NFS backend.
///
/// Starts an in-process NFS server, mounts it at the configured mount point,
/// and returns a handle that stops the server and unmounts on shutdown.
#[cfg(feature = "nfs")]
pub struct NfsMountPlugin;

#[cfg(feature = "nfs")]
impl MountPlugin for NfsMountPlugin {
    fn name(&self) -> &str {
        "nfs"
    }

    fn mount<'a>(
        &'a self,
        channel: Channel,
        config: MountConfig,
    ) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn MountHandleTrait>>> + Send + 'a>> {
        Box::pin(async move {
            let mount_point = config
                .mount_point
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let handle = mount_nfs(channel, config).await?;
            Ok(Box::new(MountHandleWrapper {
                inner: Mutex::new(Some(handle)),
                mount_point,
            }) as Box<dyn MountHandleTrait>)
        })
    }
}

#[cfg(feature = "nfs")]
async fn mount_nfs(channel: Channel, config: MountConfig) -> io::Result<ConcreteMountHandle> {
    use std::sync::Arc;

    use nfsserve::tcp::NFSTcp;

    use crate::{backend, core};

    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| io::Error::other("NFS backend requires a mount point"))?;
    let readonly = config.readonly;
    std::fs::create_dir_all(&mount_point)?;

    let fs = Arc::new(core::RemoteFs::init(channel, config).await?);

    // Start the NFS server and begin accepting connections BEFORE
    // calling mount_nfs, otherwise mount_nfs gets "Connection refused".
    let (listener, port) = backend::nfs::start_server(fs).await?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let mp = mount_point.clone();
    let join_handle = tokio::spawn(async move {
        // Spawn the NFS accept loop as a child task so the listener stays
        // alive while we unmount. tokio::select! would drop the listener
        // future BEFORE running the winning branch body, killing the NFS
        // server before diskutil can do a clean unmount.
        let server_task = tokio::spawn(async move { listener.handle_forever().await });

        let _ = shutdown_rx.await;

        // Unmount while the NFS server is still accepting connections.
        unmount_path(&mp).await;

        // Now kill the server.
        server_task.abort();
        Ok(())
    });

    // Mount now that the NFS server is accepting connections.
    // os_mount() runs a blocking OS command — use spawn_blocking to avoid
    // blocking the manager's tokio runtime.
    let mp_clone = mount_point.clone();
    let mount_result =
        tokio::task::spawn_blocking(move || backend::nfs::os_mount(port, &mp_clone, readonly))
            .await
            .map_err(|e| io::Error::other(format!("mount task panicked: {e}")))?;

    if let Err(e) = mount_result {
        let _ = shutdown_tx.send(());
        return Err(e);
    }

    Ok(ConcreteMountHandle::new(shutdown_tx, join_handle))
}

/// Maximum time to wait for the OS unmount command before giving up.
#[cfg(feature = "nfs")]
const UNMOUNT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Best-effort unmount of a filesystem path via OS utilities.
///
/// Uses `tokio::process::Command` so the call is non-blocking on the
/// tokio runtime. Times out after [`UNMOUNT_TIMEOUT`] to prevent hangs
/// if the OS unmount command blocks (e.g. kernel retrying against a
/// dead NFS server).
#[cfg(feature = "nfs")]
async fn unmount_path(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let cmd = tokio::process::Command::new("diskutil")
        .args(["unmount", "force", path.to_str().unwrap_or("")])
        .output();

    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = tokio::process::Command::new("umount")
        .args(["-f", &*path.to_string_lossy()])
        .output();

    #[cfg(windows)]
    let cmd = tokio::process::Command::new("cmd")
        .args(["/c", "net", "use", path.to_str().unwrap_or(""), "/delete"])
        .output();

    match tokio::time::timeout(UNMOUNT_TIMEOUT, cmd).await {
        Ok(Ok(output)) if output.status.success() => {
            log::debug!("unmounted {}", path.display());
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!("unmount {} failed: {}", path.display(), stderr.trim());
        }
        Ok(Err(e)) => {
            log::warn!("unmount {} failed: {e}", path.display());
        }
        Err(_) => {
            log::warn!(
                "unmount {} timed out after {:?}",
                path.display(),
                UNMOUNT_TIMEOUT
            );
        }
    }
}

// FUSE

/// Mount plugin for the FUSE backend.
///
/// Creates a FUSE session at the configured mount point and returns a handle
/// that tears down the session on shutdown.
#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
pub struct FuseMountPlugin;

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
impl MountPlugin for FuseMountPlugin {
    fn name(&self) -> &str {
        "fuse"
    }

    fn mount<'a>(
        &'a self,
        channel: Channel,
        config: MountConfig,
    ) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn MountHandleTrait>>> + Send + 'a>> {
        Box::pin(async move {
            let mount_point = config
                .mount_point
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let handle = mount_fuse(channel, config).await?;
            Ok(Box::new(MountHandleWrapper {
                inner: Mutex::new(Some(handle)),
                mount_point,
            }) as Box<dyn MountHandleTrait>)
        })
    }
}

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
async fn mount_fuse(channel: Channel, config: MountConfig) -> io::Result<ConcreteMountHandle> {
    use std::sync::Arc;

    use tokio::runtime::Handle;

    use crate::{backend, core};

    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| io::Error::other("FUSE backend requires a mount point"))?;
    let readonly = config.readonly;
    std::fs::create_dir_all(&mount_point)?;
    let fs = core::RemoteFs::init(channel, config).await?;
    let rt = Arc::new(core::Runtime::with_fs(Handle::current(), fs));

    // fuser::spawn_mount2 is blocking — use spawn_blocking
    let session =
        tokio::task::spawn_blocking(move || backend::fuse::mount(rt, &mount_point, readonly))
            .await
            .map_err(|e| io::Error::other(format!("fuse mount task panicked: {e}")))??;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        // Keep the BackgroundSession alive until shutdown signal.
        let _session = session;
        let _ = shutdown_rx.await;
        Ok(())
    });

    Ok(ConcreteMountHandle::new(shutdown_tx, join_handle))
}

// macOS FileProvider

/// Mount plugin for the macOS FileProvider backend.
///
/// Registers a FileProvider domain with macOS. The OS manages the actual
/// mount point (visible in Finder sidebar). The returned handle is detached
/// since no foreground process is needed.
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub struct FileProviderMountPlugin;

#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
impl MountPlugin for FileProviderMountPlugin {
    fn name(&self) -> &str {
        "macos-file-provider"
    }

    fn mount<'a>(
        &'a self,
        channel: Channel,
        config: MountConfig,
    ) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn MountHandleTrait>>> + Send + 'a>> {
        Box::pin(async move {
            let (handle, mount_point) = mount_file_provider(channel, config).await?;
            Ok(Box::new(MountHandleWrapper {
                inner: Mutex::new(Some(handle)),
                mount_point,
            }) as Box<dyn MountHandleTrait>)
        })
    }
}

#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
async fn mount_file_provider(
    channel: Channel,
    config: MountConfig,
) -> io::Result<(ConcreteMountHandle, String)> {
    use std::sync::Arc;

    use tokio::runtime::Handle;

    use crate::{backend, core};

    // Build domain metadata by flattening config fields into the extra
    // map. The extra map already has connection_id, destination, and
    // log_level (injected by the manager). Add remote_root and readonly
    // from the config so register_domain has everything it needs.
    let mut domain_meta = config.extra.clone();
    if let Some(ref root) = config.remote_root {
        domain_meta.insert("remote_root".into(), root.to_string());
    }
    if config.readonly {
        domain_meta.insert("readonly".into(), "true".into());
    }

    let fs = core::RemoteFs::init(channel, config).await?;
    let rt = Arc::new(core::Runtime::with_fs(Handle::current(), fs));

    let (domain_id, _display_name) =
        backend::macos_file_provider::register_domain(rt, &domain_meta)?;
    let mount_point = backend::macos_file_provider::cloud_storage_path_for_domain(&domain_id)
        .await?
        .to_string_lossy()
        .into_owned();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        let _ = shutdown_rx.await;
        Ok(())
    });

    Ok((
        ConcreteMountHandle::new(shutdown_tx, join_handle).detach(),
        mount_point,
    ))
}

// Windows Cloud Files

/// Mount plugin for the Windows Cloud Files backend.
///
/// Registers a Cloud Files sync root at the configured mount point and
/// returns a handle that unregisters the sync root on shutdown.
#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
pub struct CloudFilesMountPlugin;

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
impl MountPlugin for CloudFilesMountPlugin {
    fn name(&self) -> &str {
        "windows-cloud-files"
    }

    fn mount<'a>(
        &'a self,
        channel: Channel,
        config: MountConfig,
    ) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn MountHandleTrait>>> + Send + 'a>> {
        Box::pin(async move {
            let mount_point = config
                .mount_point
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let handle = mount_cloud_files(channel, config).await?;
            Ok(Box::new(MountHandleWrapper {
                inner: Mutex::new(Some(handle)),
                mount_point,
            }) as Box<dyn MountHandleTrait>)
        })
    }
}

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
async fn mount_cloud_files(
    channel: Channel,
    config: MountConfig,
) -> io::Result<ConcreteMountHandle> {
    use std::sync::Arc;

    use tokio::runtime::Handle;

    use crate::{backend, core};

    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| io::Error::other("Windows Cloud Files backend requires a mount point"))?;

    let watcher_channel = channel.clone();
    let fs = Arc::new(core::RemoteFs::init(channel, config).await?);

    let guard = backend::windows_cloud_files::mount(
        Handle::current(),
        Arc::clone(&fs),
        watcher_channel,
        &mount_point,
    )?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        // Keep the connection guard alive until shutdown signal.
        let _guard = guard;
        let _ = shutdown_rx.await;
        // Unregister sync root on shutdown.
        let _ = backend::windows_cloud_files::unmount();
        Ok(())
    });

    Ok(ConcreteMountHandle::new(shutdown_tx, join_handle))
}
