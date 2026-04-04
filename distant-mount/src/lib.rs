mod backend;
mod core;
pub mod plugin;

use std::io;
use std::sync::Arc;

use distant_core::Channel;
use tokio::runtime::Handle;

pub use backend::MountBackend;
pub use core::{CacheConfig, MountConfig, MountHandle};

// Re-export Windows Cloud Files utilities for the binary crate.
#[cfg(all(target_os = "windows", feature = "windows-cloud-files"))]
pub mod windows_cloud_files {
    pub use crate::backend::windows_cloud_files::{unmount, unmount_path};
}

// Re-export macOS utilities for the binary crate.
#[cfg(all(target_os = "macos", feature = "macos-file-provider"))]
pub mod macos {
    pub use crate::backend::macos_file_provider::utils::{
        app_group_container_path, is_file_provider_extension, is_running_in_app_bundle,
    };
    pub use crate::backend::macos_file_provider::{
        ChannelResolver, DomainInfo, init_file_provider, list_file_provider_domains,
        register_file_provider_classes, remove_all_file_provider_domains,
        remove_file_provider_domain_for_destination,
    };
}

/// Mount a remote filesystem using the specified backend.
///
/// The `handle` is the Tokio runtime handle used by sync-callback backends
/// (FUSE, FileProvider) to bridge into async code. Async-native backends
/// (NFS) run on the current runtime.
///
/// Returns a [`MountHandle`] that can be used to unmount or wait for the
/// mount to end.
pub async fn mount(
    handle: Handle,
    channel: Channel,
    config: MountConfig,
    backend: MountBackend,
) -> io::Result<MountHandle> {
    match backend {
        #[cfg(all(
            feature = "fuse",
            any(target_os = "linux", target_os = "freebsd", target_os = "macos")
        ))]
        MountBackend::Fuse => mount_fuse(handle, channel, config).await,
        #[cfg(feature = "nfs")]
        MountBackend::Nfs => mount_nfs(channel, config).await,
        #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
        MountBackend::WindowsCloudFiles => mount_cloud_files(handle, channel, config).await,
        #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
        MountBackend::MacosFileProvider => mount_file_provider(handle, channel, config).await,
    }
}

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
async fn mount_fuse(
    handle: Handle,
    channel: Channel,
    config: MountConfig,
) -> io::Result<MountHandle> {
    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| io::Error::other("FUSE backend requires a mount point"))?;
    let readonly = config.readonly;
    std::fs::create_dir_all(&mount_point)?;
    let fs = core::RemoteFs::init(channel, config).await?;
    let rt = Arc::new(core::Runtime::with_fs(handle, fs));

    let session = backend::fuse::mount(rt, &mount_point, readonly)?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        // Keep the BackgroundSession alive until shutdown signal.
        let _session = session;
        let _ = shutdown_rx.await;
        Ok(())
    });

    Ok(MountHandle::new(shutdown_tx, join_handle))
}

#[cfg(feature = "nfs")]
async fn mount_nfs(channel: Channel, config: MountConfig) -> io::Result<MountHandle> {
    use nfsserve::tcp::NFSTcp;

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

    Ok(MountHandle::new(shutdown_tx, join_handle))
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

#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
async fn mount_file_provider(
    handle: Handle,
    channel: Channel,
    config: MountConfig,
) -> io::Result<MountHandle> {
    let extra = config.extra.clone();
    let fs = core::RemoteFs::init(channel, config).await?;
    let rt = Arc::new(core::Runtime::with_fs(handle, fs));

    let _domain_id = backend::macos_file_provider::register_domain(rt, &extra)?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        let _ = shutdown_rx.await;
        Ok(())
    });

    Ok(MountHandle::new(shutdown_tx, join_handle).detach())
}

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
async fn mount_cloud_files(
    handle: Handle,
    channel: Channel,
    config: MountConfig,
) -> io::Result<MountHandle> {
    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| io::Error::other("Windows Cloud Files backend requires a mount point"))?;

    let watcher_channel = channel.clone();
    let fs = Arc::new(core::RemoteFs::init(channel, config).await?);

    let guard = backend::windows_cloud_files::mount(
        handle.clone(),
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

    Ok(MountHandle::new(shutdown_tx, join_handle))
}
