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
