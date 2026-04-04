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
            let handle = mount_nfs(channel, config).await?;
            let mount_point = String::new();
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
        let result = tokio::select! {
            result = listener.handle_forever() => result,
            _ = shutdown_rx => Ok(()),
        };
        unmount_path(&mp);
        result
    });

    // Mount now that the NFS server is accepting connections.
    // (The TCP socket is listening at the kernel level after bind().)
    if let Err(e) = backend::nfs::os_mount(port, &mount_point, readonly) {
        let _ = shutdown_tx.send(());
        return Err(e);
    }

    Ok(ConcreteMountHandle::new(shutdown_tx, join_handle))
}

/// Best-effort unmount of a filesystem path via OS utilities.
#[cfg(feature = "nfs")]
fn unmount_path(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("diskutil")
        .args(["unmount", path.to_str().unwrap_or("")])
        .output();

    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("umount").arg(path).output();

    #[cfg(windows)]
    let result = std::process::Command::new("cmd")
        .args(["/c", "net", "use", path.to_str().unwrap_or(""), "/delete"])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            log::debug!("unmounted {}", path.display());
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!("unmount {} failed: {}", path.display(), stderr.trim());
        }
        Err(e) => {
            log::warn!("unmount {} failed: {e}", path.display());
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
            let handle = mount_fuse(channel, config).await?;
            let mount_point = String::new();
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

    let session = backend::fuse::mount(rt, &mount_point, readonly)?;

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
            let handle = mount_file_provider(channel, config).await?;
            // FileProvider doesn't have a traditional mount point -- macOS
            // manages the CloudStorage directory.
            let mount_point = String::new();
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
) -> io::Result<ConcreteMountHandle> {
    use std::sync::Arc;

    use tokio::runtime::Handle;

    use crate::{backend, core};

    let extra = config.extra.clone();
    let fs = core::RemoteFs::init(channel, config).await?;
    let rt = Arc::new(core::Runtime::with_fs(Handle::current(), fs));

    let _domain_id = backend::macos_file_provider::register_domain(rt, &extra)?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        let _ = shutdown_rx.await;
        Ok(())
    });

    Ok(ConcreteMountHandle::new(shutdown_tx, join_handle).detach())
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
            let handle = mount_cloud_files(channel, config).await?;
            let mount_point = String::new();
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
