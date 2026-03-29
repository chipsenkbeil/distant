use std::path::PathBuf;
use std::time::Duration;

use distant_core::net::common::Map;
use distant_core::protocol::RemotePath;

/// Configuration for mounting a filesystem.
#[derive(Clone, Debug, Default)]
pub struct MountConfig {
    /// Local mount point path.
    ///
    /// Required for FUSE, NFS, and Windows Cloud Files backends. Not used by
    /// the macOS FileProvider backend (macOS manages the CloudStorage folder).
    pub mount_point: Option<PathBuf>,

    /// Remote directory to expose (defaults to the server's current working
    /// directory when `None`).
    pub remote_root: Option<RemotePath>,

    /// Mount as read-only.
    pub readonly: bool,

    /// Cache configuration.
    pub cache: CacheConfig,

    /// Backend-specific key-value data.
    ///
    /// For FileProvider: expects `connection_id` and `destination` keys.
    /// For other backends: currently unused.
    pub extra: Map,
}

/// Cache tuning parameters for a mounted filesystem.
///
/// Controls time-to-live durations and maximum capacities for the attribute,
/// directory listing, and read caches. Shorter TTLs give more up-to-date
/// views of remote state at the cost of additional round trips.
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
