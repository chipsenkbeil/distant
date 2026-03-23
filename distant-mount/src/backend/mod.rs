//! Mount backend trait and platform-specific implementations.

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
pub(crate) mod fuse;

#[cfg(all(
    feature = "nfs",
    any(
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd"
    )
))]
#[allow(dead_code)]
pub(crate) mod nfs;

#[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
pub(crate) mod windows_cloud_files;

// macOS FileProvider backend — requires .appex inside .app bundle.
// See macos_file_provider.rs module docs for architecture details.
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
pub(crate) mod macos_file_provider;
