//! Mount-related protocol types shared across distant crates.
//!
//! Provides configuration, resource identification, and status types used by
//! the mount subsystem and the manager's resource tracking.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::net::common::Map;
use crate::protocol::RemotePath;

/// Identifies a type of managed resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    /// A distant connection to a remote server.
    Connection,
    /// A TCP tunnel (forward or reverse).
    Tunnel,
    /// A mounted remote filesystem.
    Mount,
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection => write!(f, "connection"),
            Self::Tunnel => write!(f, "tunnel"),
            Self::Mount => write!(f, "mount"),
        }
    }
}

/// Configuration for mounting a filesystem.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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

/// Lifecycle state of a mount tracked by the manager.
///
/// Distinct from [`crate::net::client::ConnectionState`] so the user
/// can tell at a glance which subsystem they're looking at and so
/// mount-side `Failed` (terminal) is distinguishable from the
/// transient connection-side `Disconnected` (recoverable).
///
/// State machine:
/// ```text
///                ┌──────────────┐ connection drop ┌──────────────┐
///     Active ───►│ Reconnecting ├────────────────►│ Disconnected │
///          ▲     └──────┬───────┘                 └──────┬───────┘
///          │            │ reconnect succeeded            │
///          └────────────┘                                │
///                                                        │ permanent
///                                                        ▼
///                                                   ┌────────┐
///                                                   │ Failed │
///                                                   └────────┘
/// ```
///
/// `Failed` is terminal — the only exit is to unmount and remount.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MountStatus {
    /// Mount is healthy and serving requests.
    #[default]
    Active,
    /// Underlying connection is reconnecting; the mount will resume
    /// once the connection comes back.
    Reconnecting,
    /// Underlying connection is gone; the mount cannot serve
    /// requests but may still recover via reconnect.
    Disconnected,
    /// Mount has failed permanently. The only exit is to unmount
    /// and remount.
    Failed { reason: String },
}

/// Describes an active mount managed by the distant manager.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MountInfo {
    /// Unique mount identifier.
    pub id: u32,

    /// Connection this mount uses.
    pub connection_id: u32,

    /// Backend name (e.g., "nfs", "fuse", "macos-file-provider", "windows-cloud-files").
    pub backend: String,

    /// Local mount point path.
    pub mount_point: String,

    /// Remote root directory that is mounted.
    pub remote_root: String,

    /// Whether the mount is read-only.
    pub readonly: bool,

    /// Current lifecycle state of the mount.
    pub status: MountStatus,
}

#[cfg(test)]
mod mount_status_tests {
    use super::*;

    #[test]
    fn mount_status_default_is_active() {
        assert_eq!(MountStatus::default(), MountStatus::Active);
    }

    #[test]
    fn mount_status_active_serializes_to_state_active() {
        let json = serde_json::to_string(&MountStatus::Active).unwrap();
        assert_eq!(json, r#"{"state":"active"}"#);
    }

    #[test]
    fn mount_status_reconnecting_serializes_to_state_reconnecting() {
        let json = serde_json::to_string(&MountStatus::Reconnecting).unwrap();
        assert_eq!(json, r#"{"state":"reconnecting"}"#);
    }

    #[test]
    fn mount_status_disconnected_serializes_to_state_disconnected() {
        let json = serde_json::to_string(&MountStatus::Disconnected).unwrap();
        assert_eq!(json, r#"{"state":"disconnected"}"#);
    }

    #[test]
    fn mount_status_failed_includes_reason_in_payload() {
        let status = MountStatus::Failed {
            reason: "fuse session ended".to_string(),
        };
        let value: serde_json::Value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["state"], "failed");
        assert_eq!(value["reason"], "fuse session ended");
    }

    #[test]
    fn mount_status_round_trips_through_json_for_every_variant() {
        let variants = [
            MountStatus::Active,
            MountStatus::Reconnecting,
            MountStatus::Disconnected,
            MountStatus::Failed {
                reason: "test failure".to_string(),
            },
        ];
        for original in variants {
            let json = serde_json::to_string(&original).unwrap();
            let decoded: MountStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn mount_status_should_reject_unknown_state() {
        let result = serde_json::from_str::<MountStatus>(r#"{"state":"bogus"}"#);
        assert!(result.is_err(), "expected unknown state to fail to parse");
    }

    #[test]
    fn mount_status_should_reject_failed_without_reason() {
        // The Failed variant requires the reason field — missing it
        // should fail to parse rather than silently default to "".
        let result = serde_json::from_str::<MountStatus>(r#"{"state":"failed"}"#);
        assert!(
            result.is_err(),
            "expected Failed without reason to fail to parse"
        );
    }
}
