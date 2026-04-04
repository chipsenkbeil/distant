//! Mount plugin trait for filesystem mount backends.
//!
//! Provides the [`MountPlugin`] and [`MountHandle`] traits that mount backends
//! implement. The manager uses these traits to orchestrate mount lifecycle
//! without coupling to any specific backend.

use std::future::Future;
use std::io;
use std::pin::Pin;

use crate::Channel;
use crate::protocol::MountConfig;

/// Object-safe plugin interface for mount backends.
///
/// Each implementation wraps a specific mount technology (FUSE, NFS,
/// FileProvider, Cloud Files) behind a uniform async API. The manager
/// calls [`mount`](MountPlugin::mount) to create a mount and receives a
/// [`MountHandle`] for lifecycle control.
pub trait MountPlugin: Send + Sync {
    /// Human-readable name for this mount backend (e.g. "fuse", "nfs").
    fn name(&self) -> &str;

    /// Mount a remote filesystem over the given channel.
    ///
    /// Returns a handle that can be used to query the mount point and
    /// trigger an unmount.
    #[allow(clippy::type_complexity)]
    fn mount<'a>(
        &'a self,
        channel: Channel,
        config: MountConfig,
    ) -> Pin<Box<dyn Future<Output = io::Result<Box<dyn MountHandle>>> + Send + 'a>>;
}

/// Handle to an active filesystem mount.
///
/// Returned by [`MountPlugin::mount`]. Implementations control the
/// backend-specific unmount procedure and expose the local mount point.
pub trait MountHandle: Send {
    /// Gracefully unmount the filesystem.
    fn unmount(&mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>>;

    /// Returns the local mount point path as a string.
    fn mount_point(&self) -> &str;
}
