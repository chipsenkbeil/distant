mod backend;
mod core;

use backend::MountBackend;
use core::RemoteFs;
use distant_core::Channel;
use std::io;

pub use core::{MountConfig, MountHandle};

/// Mount a remote filesystem at the given mount point using the specified backend.
///
/// Returns a [`MountHandle`] that can be used to unmount or wait for the mount
/// to end.
pub async fn mount(
    channel: Channel,
    config: MountConfig,
    backend: MountBackend,
) -> io::Result<MountHandle> {
    match backend {
        #[cfg(all(
            feature = "fuse",
            any(target_os = "linux", target_os = "freebsd", target_os = "macos")
        ))]
        MountBackend::Fuse => mount_fuse(channel, config).await,
        #[cfg(feature = "nfs")]
        MountBackend::Nfs => mount_nfs(channel, config).await,
        #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
        MountBackend::WindowsCloudFiles => mount_cloud_files(channel, config).await,
        #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
        MountBackend::MacosFileProvider => mount_file_provider(channel, config).await,
    }
}

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
async fn mount_fuse(channel: Channel, config: MountConfig) -> io::Result<MountHandle> {
    use std::sync::Arc;

    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| io::Error::other("FUSE backend requires a mount point"))?;
    let fs = Arc::new(RemoteFs::init(channel, config).await?);

    let session = backend::fuse::mount(fs, &mount_point)?;

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
    use std::sync::Arc;

    use nfsserve::tcp::NFSTcp;

    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| io::Error::other("NFS backend requires a mount point"))?;

    let fs = Arc::new(RemoteFs::init(channel, config).await?);

    // Start a local server that exposes the filesystem via NFS, which
    // we will then connect to with our client implementation
    let (listener, port) = backend::nfs::start_server(fs).await?;
    backend::nfs::os_mount(port, &mount_point)?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        tokio::select! {
            result = listener.handle_forever() => result,
            _ = shutdown_rx => Ok(()),
        }
    });

    Ok(MountHandle::new(shutdown_tx, join_handle))
}

#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
async fn mount_file_provider(channel: Channel, config: MountConfig) -> io::Result<MountHandle> {
    use std::sync::Arc;

    let extra = config.extra.clone();
    let fs = Arc::new(RemoteFs::init(channel, config).await?);

    let _domain_id = backend::macos_file_provider::register_domain(fs, &extra)?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        let _ = shutdown_rx.await;
        Ok(())
    });

    Ok(MountHandle::new(shutdown_tx, join_handle).detach())
}

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
async fn mount_cloud_files(
    channel: distant_core::Channel,
    config: MountConfig,
) -> io::Result<MountHandle> {
    use std::sync::Arc;

    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| io::Error::other("Windows Cloud Files backend requires a mount point"))?;
    let fs = Arc::new(RemoteFs::init(channel, config).await?);

    let session = backend::windows_cloud_files::mount(fs, &mount_point)?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        // Keep the Session alive until shutdown signal.
        let _session = session;
        let _ = shutdown_rx.await;
        // Unregister sync root on shutdown.
        let _ = backend::windows_cloud_files::unmount();
        Ok(())
    });

    Ok(MountHandle::new(shutdown_tx, join_handle))
}
