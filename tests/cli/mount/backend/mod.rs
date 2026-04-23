//! Backend-specific mount integration tests.
//!
//! Each submodule targets a single mount backend and verifies behavior that
//! is unique to that backend (e.g., filesystem type in the mount table,
//! macOS FileProvider domain registration, Windows Cloud Files sync root).

#[cfg(feature = "mount-nfs")]
mod nfs;

#[cfg(all(
    feature = "mount-fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
mod fuse;

#[cfg(all(feature = "mount-macos-file-provider", target_os = "macos"))]
mod macos_file_provider;

#[cfg(all(feature = "mount-windows-cloud-files", target_os = "windows"))]
mod windows_cloud_files;
