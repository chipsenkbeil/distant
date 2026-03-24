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

pub use config::{CacheConfig, MountBackend, MountConfig, MountHandle, ParseMountBackendError};
pub use remote_fs::RemoteFs;

/// Callback type that resolves a connection ID and destination string into
/// a [`distant_core::Channel`] by communicating with the distant manager.
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub type ChannelResolver =
    Box<dyn Fn(u32, &str) -> std::io::Result<distant_core::Channel> + Send + Sync>;

/// Stores the Tokio runtime handle and channel resolver needed by the
/// `.appex` FileProvider extension bootstrap flow.
///
/// Must be called once from the host process before macOS instantiates the
/// `DistantFileProvider` class via `initWithDomain:`.
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub fn init_file_provider(rt: tokio::runtime::Handle, resolve_channel: ChannelResolver) {
    backend::macos_file_provider::init(rt, resolve_channel);
}

/// Returns `true` if this process is running as a macOS `.appex` FileProvider extension.
///
/// Checks `NSBundle.mainBundle.bundlePath` for a `.appex` suffix, which is
/// Apple's standard approach for distinguishing `.app` from `.appex` bundles.
///
/// Always returns `false` on non-macOS platforms or when the `macos-file-provider`
/// feature is not enabled.
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub fn is_file_provider_extension() -> bool {
    use objc2_foundation::NSBundle;
    NSBundle::mainBundle()
        .bundlePath()
        .to_string()
        .ends_with(".appex")
}

/// Mount a remote filesystem at the given mount point using the specified backend.
///
/// Returns a [`MountHandle`] that can be used to unmount or wait for the mount
/// to end.
///
/// # Errors
///
/// Returns an error if the [`RemoteFs`] fails to initialize (e.g., the initial
/// `system_info` call fails) or the backend-specific mount operation fails.
#[allow(unused_variables)]
pub fn mount(
    rt: tokio::runtime::Handle,
    channel: distant_core::Channel,
    config: MountConfig,
    backend: MountBackend,
) -> std::io::Result<MountHandle> {
    match backend {
        #[cfg(all(
            feature = "fuse",
            any(target_os = "linux", target_os = "freebsd", target_os = "macos")
        ))]
        MountBackend::Fuse => mount_fuse(rt, channel, config),
        #[cfg(feature = "nfs")]
        MountBackend::Nfs => mount_nfs(rt, channel, config),
        #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
        MountBackend::WindowsCloudFiles => mount_cloud_files(rt, channel, config),
        #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
        MountBackend::MacosFileProvider => mount_file_provider(rt, channel, config),
        // When no backends are compiled the enum is uninhabited.
        #[allow(unreachable_patterns)]
        _ => unreachable!(),
    }
}

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
fn mount_fuse(
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

#[cfg(feature = "nfs")]
fn mount_nfs(
    rt: tokio::runtime::Handle,
    channel: distant_core::Channel,
    config: MountConfig,
) -> std::io::Result<MountHandle> {
    use std::sync::Arc;

    use nfsserve::tcp::NFSTcp;

    let mount_point = config.mount_point.clone();

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

#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
fn mount_file_provider(
    rt: tokio::runtime::Handle,
    channel: distant_core::Channel,
    config: MountConfig,
) -> std::io::Result<MountHandle> {
    use std::sync::Arc;

    let extra = config.extra.clone();
    let fs = Arc::new(RemoteFs::new(rt, channel, config)?);

    backend::macos_file_provider::register_domain(fs, &extra)?;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let join_handle = tokio::spawn(async move {
        let _ = shutdown_rx.await;
        Ok(())
    });

    Ok(MountHandle::new(shutdown_tx, join_handle).set_needs_foreground(false))
}

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
fn mount_cloud_files(
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
