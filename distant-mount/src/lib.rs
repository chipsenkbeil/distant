#[allow(dead_code)]
mod cache;
mod config;
#[allow(dead_code)]
mod inode;
#[allow(dead_code)]
mod remote_fs;
#[allow(dead_code)]
mod write_buffer;

pub mod backend;

pub use config::{CacheConfig, MountConfig, MountHandle};
pub use remote_fs::RemoteFs;

/// Mount a remote filesystem at the given mount point.
///
/// Returns a [`MountHandle`] that can be used to unmount or wait for the mount
/// to end.
///
/// # Errors
///
/// Returns an error if the [`RemoteFs`] fails to initialize (e.g., the initial
/// `system_info` call fails) or the FUSE mount fails (e.g., missing permissions
/// or the mount point does not exist).
#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
pub fn mount(
    rt: tokio::runtime::Handle,
    channel: distant_core::Channel,
    config: MountConfig,
) -> std::io::Result<MountHandle> {
    use std::sync::Arc;

    let mount_point = config.mount_point.clone();
    let fs = Arc::new(RemoteFs::new(rt, channel, config)?);

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

/// Mount a remote filesystem at the given mount point using a localhost NFS server.
///
/// Returns a [`MountHandle`] that can be used to unmount or wait for the mount
/// to end. A localhost NFSv3 server is started on a random port and the OS-native
/// `mount_nfs` command attaches it to the mount point.
///
/// This variant is selected on OpenBSD and NetBSD where FUSE is not available.
///
/// # Errors
///
/// Returns an error if the [`RemoteFs`] fails to initialize, the NFS server
/// cannot bind to a local port, or the OS mount command fails.
#[cfg(all(
    feature = "nfs",
    any(target_os = "openbsd", target_os = "netbsd"),
    not(all(
        feature = "fuse",
        any(target_os = "linux", target_os = "freebsd", target_os = "macos")
    ))
))]
pub fn mount(
    rt: tokio::runtime::Handle,
    channel: distant_core::Channel,
    config: MountConfig,
) -> std::io::Result<MountHandle> {
    use std::sync::Arc;

    let mount_point = config.mount_point.clone();
    use nfsserve::tcp::NFSTcp;

    let rt_handle = rt.clone();
    let fs = Arc::new(RemoteFs::new(rt, channel, config)?);

    let (listener, port) = rt_handle.block_on(backend::nfs::start_server(fs))?;
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

/// Mount a remote filesystem at the given mount point using Windows Cloud Files.
///
/// Returns a [`MountHandle`] that can be used to unmount or wait for the mount
/// to end. The mount point directory must already exist and will be registered
/// as a Cloud Files sync root with native File Explorer integration.
///
/// # Errors
///
/// Returns an error if the [`RemoteFs`] fails to initialize (e.g., the initial
/// `system_info` call fails), the sync root registration fails, or the Cloud
/// Filter session connection fails.
/// Mount a remote filesystem using the macOS FileProvider framework.
///
/// Returns a [`MountHandle`] that keeps the FileProvider domain active.
/// The system launches the `.appex` extension process when the domain
/// is accessed in Finder.
///
/// Unlike FUSE, this provides native Finder integration with placeholder
/// files, but requires a `.app` bundle containing the `.appex` extension.
///
/// # Errors
///
/// Returns an error if the [`RemoteFs`] fails to initialize.
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub fn mount_file_provider(
    rt: tokio::runtime::Handle,
    channel: distant_core::Channel,
    config: MountConfig,
) -> std::io::Result<MountHandle> {
    use std::sync::Arc;

    let fs = Arc::new(RemoteFs::new(rt, channel, config)?);

    backend::macos_file_provider::register_domain(fs)?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        let _ = shutdown_rx.await;
        Ok(())
    });

    Ok(MountHandle::new(shutdown_tx, join_handle))
}

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
pub fn mount(
    rt: tokio::runtime::Handle,
    channel: distant_core::Channel,
    config: MountConfig,
) -> std::io::Result<MountHandle> {
    use std::sync::Arc;

    let mount_point = config.mount_point.clone();
    let fs = Arc::new(RemoteFs::new(rt, channel, config)?);

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
