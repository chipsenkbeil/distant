//! Mount configuration and lifecycle handle types.
//!
//! Provides [`MountConfig`] for specifying how a remote filesystem should be
//! mounted locally, [`CacheConfig`] for tuning the attribute, directory listing,
//! and read caches, and [`MountHandle`] for controlling the lifetime of an
//! active mount.

use std::io;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use std::{error, fmt};

use distant_core::protocol::RemotePath;

/// Configuration for a FUSE mount of a remote filesystem.
///
/// Specifies the local mount point, the remote directory to expose, access
/// mode, and caching behavior.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
///
/// use distant_mount::{CacheConfig, MountConfig};
///
/// let config = MountConfig {
///     mount_point: PathBuf::from("/mnt/remote"),
///     remote_root: None,
///     readonly: false,
///     cache: CacheConfig::default(),
/// };
/// ```
#[derive(Clone, Debug)]
pub struct MountConfig {
    /// Local mount point path.
    pub mount_point: PathBuf,

    /// Remote directory to expose (defaults to the server's current working
    /// directory when `None`).
    pub remote_root: Option<RemotePath>,

    /// Mount as read-only.
    pub readonly: bool,

    /// Cache configuration.
    pub cache: CacheConfig,
}

/// Cache tuning parameters for a mounted filesystem.
///
/// Controls time-to-live durations and maximum capacities for the attribute,
/// directory listing, and read caches. Shorter TTLs give more up-to-date
/// views of remote state at the cost of additional round trips.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use distant_mount::CacheConfig;
///
/// let config = CacheConfig {
///     attr_ttl: Duration::from_millis(500),
///     dir_ttl: Duration::from_millis(500),
///     read_ttl: Duration::from_secs(10),
///     attr_capacity: 5000,
///     dir_capacity: 500,
///     read_capacity: 50,
/// };
/// ```
#[derive(Clone, Debug)]
pub struct CacheConfig {
    /// Attribute cache TTL.
    pub attr_ttl: Duration,

    /// Directory listing cache TTL.
    pub dir_ttl: Duration,

    /// Read cache TTL.
    pub read_ttl: Duration,

    /// Maximum number of cached attributes.
    pub attr_capacity: usize,

    /// Maximum number of cached directory listings.
    pub dir_capacity: usize,

    /// Maximum number of cached file contents.
    pub read_capacity: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            attr_ttl: Duration::from_secs(1),
            dir_ttl: Duration::from_secs(1),
            read_ttl: Duration::from_secs(30),
            attr_capacity: 10_000,
            dir_capacity: 1_000,
            read_capacity: 100,
        }
    }
}

/// Selects which mount backend to use.
///
/// Each variant is only available when its corresponding feature and
/// platform requirements are met. Use [`MountBackend::available_backends`]
/// to discover which backends are compiled in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountBackend {
    #[cfg(all(
        feature = "fuse",
        any(target_os = "linux", target_os = "freebsd", target_os = "macos")
    ))]
    Fuse,

    #[cfg(feature = "nfs")]
    Nfs,

    #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
    WindowsCloudFiles,

    #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
    MacosFileProvider,
}

impl MountBackend {
    /// Returns the list of backends available in this build.
    pub fn available_backends() -> &'static [&'static str] {
        &[
            #[cfg(all(
                feature = "fuse",
                any(target_os = "linux", target_os = "freebsd", target_os = "macos")
            ))]
            "fuse",
            #[cfg(feature = "nfs")]
            "nfs",
            #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
            "windows-cloud-files",
            #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
            "macos-file-provider",
        ]
    }
}

impl Default for MountBackend {
    fn default() -> Self {
        #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
        {
            if is_running_in_app_bundle() {
                return Self::MacosFileProvider;
            }
        }

        #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
        {
            return Self::WindowsCloudFiles;
        }

        #[cfg(feature = "nfs")]
        {
            return Self::Nfs;
        }

        #[cfg(all(
            feature = "fuse",
            any(target_os = "linux", target_os = "freebsd", target_os = "macos")
        ))]
        {
            return Self::Fuse;
        }

        #[allow(unreachable_code)]
        {
            // This branch is only reached when no mount backends are compiled.
            // The enum is empty in that case, so constructing a value is impossible.
            // The CLI won't expose mount commands without at least one backend feature.
            panic!("no mount backend available — enable at least one mount feature")
        }
    }
}

impl fmt::Display for MountBackend {
    #[allow(unused_variables)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(all(
                feature = "fuse",
                any(target_os = "linux", target_os = "freebsd", target_os = "macos")
            ))]
            Self::Fuse => f.write_str("fuse"),
            #[cfg(feature = "nfs")]
            Self::Nfs => f.write_str("nfs"),
            #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
            Self::WindowsCloudFiles => f.write_str("windows-cloud-files"),
            #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
            Self::MacosFileProvider => f.write_str("macos-file-provider"),
            #[allow(unreachable_patterns)]
            _ => unreachable!(),
        }
    }
}

/// Error returned when parsing an invalid [`MountBackend`] string.
#[derive(Clone, Debug)]
pub struct ParseMountBackendError(String);

impl fmt::Display for ParseMountBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown mount backend {:?}, available: {:?}",
            self.0,
            MountBackend::available_backends()
        )
    }
}

impl error::Error for ParseMountBackendError {}

impl FromStr for MountBackend {
    type Err = ParseMountBackendError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            #[cfg(all(
                feature = "fuse",
                any(target_os = "linux", target_os = "freebsd", target_os = "macos")
            ))]
            "fuse" => Ok(Self::Fuse),
            #[cfg(feature = "nfs")]
            "nfs" => Ok(Self::Nfs),
            #[cfg(all(feature = "windows-cloud-files", target_os = "windows"))]
            "windows-cloud-files" => Ok(Self::WindowsCloudFiles),
            #[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
            "macos-file-provider" => Ok(Self::MacosFileProvider),
            _ => Err(ParseMountBackendError(s.to_string())),
        }
    }
}

/// Detects whether this process is running inside a `.app` bundle.
///
/// Uses `NSBundle.mainBundle.bundleIdentifier` — non-nil means bundled.
/// This is Apple's recommended approach (more reliable than path string matching).
#[cfg(all(feature = "macos-file-provider", target_os = "macos"))]
fn is_running_in_app_bundle() -> bool {
    use objc2_foundation::NSBundle;
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// Handle to an active filesystem mount.
///
/// Provides methods to gracefully unmount or wait for an externally-driven
/// unmount (e.g. Ctrl+C). Dropping the handle without calling [`unmount`] or
/// [`wait`] will send the shutdown signal, but will not block on the
/// background task completing.
///
/// [`unmount`]: MountHandle::unmount
/// [`wait`]: MountHandle::wait
#[derive(Debug)]
pub struct MountHandle {
    /// Sender half of the shutdown signal. Consumed by [`unmount`] or [`Drop`].
    ///
    /// [`unmount`]: MountHandle::unmount
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,

    /// Join handle for the background mount task. Consumed by [`unmount`] or
    /// [`wait`].
    ///
    /// [`unmount`]: MountHandle::unmount
    /// [`wait`]: MountHandle::wait
    join_handle: Option<tokio::task::JoinHandle<io::Result<()>>>,
}

impl MountHandle {
    /// Creates a new mount handle from the given shutdown channel and task
    /// join handle.
    pub fn new(
        shutdown_tx: tokio::sync::oneshot::Sender<()>,
        join_handle: tokio::task::JoinHandle<io::Result<()>>,
    ) -> Self {
        Self {
            shutdown_tx: Some(shutdown_tx),
            join_handle: Some(join_handle),
        }
    }

    /// Sends a shutdown signal to the mount and waits for the background task
    /// to complete.
    ///
    /// # Errors
    ///
    /// Returns an error if the background mount task failed or panicked.
    pub async fn unmount(mut self) -> io::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            // The receiver may already be dropped if the mount exited on its
            // own; that is not an error.
            let _ = tx.send(());
        }

        self.await_join_handle().await
    }

    /// Waits for the mount to terminate without sending a shutdown signal.
    ///
    /// This is useful when unmounting is driven externally (e.g. via `fusermount -u`
    /// or Ctrl+C).
    ///
    /// # Errors
    ///
    /// Returns an error if the background mount task failed or panicked.
    pub async fn wait(mut self) -> io::Result<()> {
        self.await_join_handle().await
    }

    /// Awaits the join handle, converting a join error into an [`io::Error`].
    async fn await_join_handle(&mut self) -> io::Result<()> {
        match self.join_handle.take() {
            Some(handle) => handle
                .await
                .unwrap_or_else(|err| Err(io::Error::other(err))),
            None => Ok(()),
        }
    }
}

impl Drop for MountHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}
