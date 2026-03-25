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

/// App Group identifier, prefixed with the Team ID as required by the
/// provisioning profile's `application-groups` wildcard (`39C6AGD73Z.*`).
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub const APP_GROUP_ID: &str = "39C6AGD73Z.group.dev.distant";

/// Public macOS FileProvider API.
///
/// Exposes functions for the binary crate to interact with the FileProvider
/// domain lifecycle: initialising the `.appex`, registering ObjC classes,
/// resolving container paths, and cleaning up domains.
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub mod macos {
    use std::io;
    use std::path::PathBuf;

    pub use super::ChannelResolver;

    /// Stores the Tokio runtime handle and channel resolver needed by the
    /// `.appex` FileProvider extension bootstrap flow.
    ///
    /// Subsequent calls are silently ignored (the first call wins).
    pub fn init_file_provider(rt: tokio::runtime::Handle, resolve_channel: ChannelResolver) {
        crate::backend::macos_file_provider::init(rt, resolve_channel);
    }

    /// Registers FileProvider ObjC classes with the Objective-C runtime.
    ///
    /// Must be called before the XPC framework looks up
    /// `NSExtensionPrincipalClass`, as classes defined via `define_class!`
    /// are registered at runtime rather than at load time.
    pub fn register_file_provider_classes() {
        crate::backend::macos_file_provider::register_classes();
    }

    /// Returns the path to the App Group shared container.
    pub fn app_group_container_path() -> Option<PathBuf> {
        use objc2_foundation::{NSFileManager, NSString};

        let group_id = NSString::from_str(crate::APP_GROUP_ID);
        let manager = NSFileManager::defaultManager();
        let url = manager.containerURLForSecurityApplicationGroupIdentifier(&group_id)?;
        let path_ns = url.path()?;
        Some(PathBuf::from(path_ns.to_string()))
    }

    /// Returns `true` if this process is running as a macOS `.appex` FileProvider extension.
    pub fn is_file_provider_extension() -> bool {
        use objc2_foundation::NSBundle;
        NSBundle::mainBundle()
            .bundlePath()
            .to_string()
            .ends_with(".appex")
    }

    /// Removes all distant FileProvider domains and cleans up their metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if domain enumeration fails.
    pub fn remove_all_file_provider_domains() -> io::Result<()> {
        crate::backend::macos_file_provider::remove_all_domains()
    }

    /// Removes the FileProvider domain whose stored destination matches `dest`.
    ///
    /// Used during unmount-by-destination: e.g. `distant unmount ssh://root@host`.
    ///
    /// # Errors
    ///
    /// Returns an error if no matching domain is found.
    pub fn remove_file_provider_domain_for_destination(dest: &str) -> io::Result<()> {
        crate::backend::macos_file_provider::remove_domain_for_destination(dest)
    }
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

    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| std::io::Error::other("FUSE backend requires a mount point"))?;
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

    let mount_point = config
        .mount_point
        .clone()
        .ok_or_else(|| std::io::Error::other("NFS backend requires a mount point"))?;

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

    let _domain_id = backend::macos_file_provider::register_domain(fs, &extra)?;

    // FileProvider domains are persistent — macOS manages the .appex
    // lifecycle independently. Cleanup (domain removal) is handled by
    // `distant unmount`, not by dropping the MountHandle.
    // The task here is intentionally a no-op so that the domain survives
    // after `distant mount` exits.
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

    let mount_point = config.mount_point.clone().ok_or_else(|| {
        std::io::Error::other("Windows Cloud Files backend requires a mount point")
    })?;
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
