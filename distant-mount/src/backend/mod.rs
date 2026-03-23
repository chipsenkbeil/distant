//! Mount backend trait and platform-specific implementations.

#[cfg(all(
    feature = "fuse",
    any(target_os = "linux", target_os = "freebsd", target_os = "macos")
))]
pub(crate) mod fuse;
