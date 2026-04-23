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

/// Backend liveness probe result returned by [`MountHandle::probe`].
///
/// Each variant maps to a specific
/// [`MountStatus`](crate::protocol::MountStatus) transition that the
/// per-mount monitor task in the manager applies:
///
/// | Probe        | MountStatus transition           |
/// |--------------|----------------------------------|
/// | `Healthy`    | (no change)                      |
/// | `Degraded`   | (no change — informational only) |
/// | `Failed`     | → `MountStatus::Failed { reason }` |
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MountProbe {
    /// Backend is alive and serving requests.
    Healthy,
    /// Backend is alive but in a degraded state. The reason is
    /// surfaced for diagnostics; the mount stays in its current
    /// state.
    Degraded(String),
    /// Backend has failed permanently. The monitor task transitions
    /// the mount to [`MountStatus::Failed`](crate::protocol::MountStatus::Failed)
    /// and stops polling.
    Failed(String),
}

/// Handle to an active filesystem mount.
///
/// Returned by [`MountPlugin::mount`]. Implementations control the
/// backend-specific unmount procedure and expose the local mount point.
pub trait MountHandle: Send + Sync {
    /// Gracefully unmount the filesystem.
    fn unmount(&mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>>;

    /// Returns the local mount point path as a string.
    fn mount_point(&self) -> &str;

    /// Probe backend liveness.
    ///
    /// Called periodically by the manager's per-mount monitor task
    /// (default interval: 5s). Implementations should return as
    /// quickly as possible — this is hot-loop code. Return
    /// [`MountProbe::Healthy`] when nothing is wrong; the default
    /// implementation does so unconditionally for backends that
    /// haven't yet wired up a real check.
    fn probe(&self) -> MountProbe {
        MountProbe::Healthy
    }
}
