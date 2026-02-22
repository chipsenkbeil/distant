use clap::Args;
use serde::{Deserialize, Serialize};

use crate::constants;

/// Level of access control to the unix socket or windows pipe
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum AccessControl {
    /// Equates to `0o600` on Unix (read & write for owner)
    Owner,

    /// Equates to `0o660` on Unix (read & write for owner and group)
    Group,

    /// Equates to `0o666` on Unix (read & write for owner, group, and other)
    Anyone,
}

impl AccessControl {
    /// Converts into a Unix file permission octal
    pub fn into_mode(self) -> u32 {
        match self {
            Self::Owner => 0o600,
            Self::Group => 0o660,
            Self::Anyone => 0o666,
        }
    }
}

impl Default for AccessControl {
    /// Defaults to owner-only permissions
    fn default() -> Self {
        Self::Owner
    }
}

/// Represents common networking configuration
#[derive(Args, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSettings {
    /// Override the path to the Unix socket used by the manager (unix-only)
    #[clap(long)]
    pub unix_socket: Option<std::path::PathBuf>,

    /// Override the name of the local named Windows pipe used by the manager (windows-only)
    #[clap(long)]
    pub windows_pipe: Option<String>,
}

impl NetworkSettings {
    /// Merge these settings with the `other` settings. These settings take priority
    /// over the `other` settings.
    pub fn merge(&mut self, other: Self) {
        self.unix_socket = self.unix_socket.take().or(other.unix_socket);
        self.windows_pipe = self.windows_pipe.take().or(other.windows_pipe);
    }

    /// Returns option containing reference to unix path if configured
    pub fn as_unix_socket_opt(&self) -> Option<&std::path::Path> {
        self.unix_socket.as_deref()
    }

    /// Returns option containing reference to windows pipe name if configured
    pub fn as_windows_pipe_opt(&self) -> Option<&str> {
        self.windows_pipe.as_deref()
    }

    /// Returns a collection of candidate unix socket paths, which will either be
    /// the config-provided unix socket path or the default user and global socket paths
    pub fn to_unix_socket_path_candidates(&self) -> Vec<&std::path::Path> {
        match self.unix_socket.as_deref() {
            Some(path) => vec![path],
            None => vec![
                constants::user::UNIX_SOCKET_PATH.as_path(),
                constants::global::UNIX_SOCKET_PATH.as_path(),
            ],
        }
    }

    /// Returns a collection of candidate windows pipe names, which will either be
    /// the config-provided windows pipe name or the default user and global pipe names
    pub fn to_windows_pipe_name_candidates(&self) -> Vec<&str> {
        match self.windows_pipe.as_deref() {
            Some(name) => vec![name],
            None => vec![
                constants::user::WINDOWS_PIPE_NAME.as_str(),
                constants::global::WINDOWS_PIPE_NAME.as_str(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // AccessControl::into_mode
    // -------------------------------------------------------
    #[test]
    fn access_control_owner_mode() {
        assert_eq!(AccessControl::Owner.into_mode(), 0o600);
    }

    #[test]
    fn access_control_group_mode() {
        assert_eq!(AccessControl::Group.into_mode(), 0o660);
    }

    #[test]
    fn access_control_anyone_mode() {
        assert_eq!(AccessControl::Anyone.into_mode(), 0o666);
    }

    // -------------------------------------------------------
    // AccessControl::default
    // -------------------------------------------------------
    #[test]
    fn access_control_default_is_owner() {
        assert_eq!(AccessControl::default(), AccessControl::Owner);
    }

    // -------------------------------------------------------
    // AccessControl serde round-trip
    // -------------------------------------------------------
    #[test]
    fn access_control_serde_round_trip() {
        for ac in [
            AccessControl::Owner,
            AccessControl::Group,
            AccessControl::Anyone,
        ] {
            let json = serde_json::to_string(&ac).unwrap();
            let deserialized: AccessControl = serde_json::from_str(&json).unwrap();
            assert_eq!(ac, deserialized);
        }
    }

    #[test]
    fn access_control_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&AccessControl::Owner).unwrap(),
            r#""owner""#
        );
        assert_eq!(
            serde_json::to_string(&AccessControl::Group).unwrap(),
            r#""group""#
        );
        assert_eq!(
            serde_json::to_string(&AccessControl::Anyone).unwrap(),
            r#""anyone""#
        );
    }

    // -------------------------------------------------------
    // NetworkSettings::default
    // -------------------------------------------------------
    #[test]
    fn network_settings_default_has_no_overrides() {
        let ns = NetworkSettings::default();
        assert!(ns.unix_socket.is_none());
        assert!(ns.windows_pipe.is_none());
    }

    // -------------------------------------------------------
    // NetworkSettings::merge
    // -------------------------------------------------------
    #[test]
    fn merge_self_takes_priority() {
        let mut ns = NetworkSettings {
            unix_socket: Some(PathBuf::from("/my/socket")),
            windows_pipe: Some(String::from("my-pipe")),
        };
        let other = NetworkSettings {
            unix_socket: Some(PathBuf::from("/other/socket")),
            windows_pipe: Some(String::from("other-pipe")),
        };
        ns.merge(other);
        assert_eq!(ns.unix_socket, Some(PathBuf::from("/my/socket")));
        assert_eq!(ns.windows_pipe, Some(String::from("my-pipe")));
    }

    #[test]
    fn merge_falls_back_to_other_when_self_is_none() {
        let mut ns = NetworkSettings {
            unix_socket: None,
            windows_pipe: None,
        };
        let other = NetworkSettings {
            unix_socket: Some(PathBuf::from("/other/socket")),
            windows_pipe: Some(String::from("other-pipe")),
        };
        ns.merge(other);
        assert_eq!(ns.unix_socket, Some(PathBuf::from("/other/socket")));
        assert_eq!(ns.windows_pipe, Some(String::from("other-pipe")));
    }

    #[test]
    fn merge_both_none_stays_none() {
        let mut ns = NetworkSettings::default();
        let other = NetworkSettings::default();
        ns.merge(other);
        assert!(ns.unix_socket.is_none());
        assert!(ns.windows_pipe.is_none());
    }

    // -------------------------------------------------------
    // as_unix_socket_opt / as_windows_pipe_opt
    // -------------------------------------------------------
    #[test]
    fn as_unix_socket_opt_returns_some_when_set() {
        let ns = NetworkSettings {
            unix_socket: Some(PathBuf::from("/tmp/test.sock")),
            windows_pipe: None,
        };
        assert_eq!(
            ns.as_unix_socket_opt(),
            Some(std::path::Path::new("/tmp/test.sock"))
        );
    }

    #[test]
    fn as_unix_socket_opt_returns_none_when_unset() {
        let ns = NetworkSettings::default();
        assert!(ns.as_unix_socket_opt().is_none());
    }

    #[test]
    fn as_windows_pipe_opt_returns_some_when_set() {
        let ns = NetworkSettings {
            unix_socket: None,
            windows_pipe: Some(String::from("test-pipe")),
        };
        assert_eq!(ns.as_windows_pipe_opt(), Some("test-pipe"));
    }

    #[test]
    fn as_windows_pipe_opt_returns_none_when_unset() {
        let ns = NetworkSettings::default();
        assert!(ns.as_windows_pipe_opt().is_none());
    }

    // -------------------------------------------------------
    // to_unix_socket_path_candidates
    // -------------------------------------------------------
    #[test]
    fn unix_socket_candidates_when_configured() {
        let ns = NetworkSettings {
            unix_socket: Some(PathBuf::from("/custom/socket.sock")),
            windows_pipe: None,
        };
        let candidates = ns.to_unix_socket_path_candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], std::path::Path::new("/custom/socket.sock"));
    }

    #[test]
    fn unix_socket_candidates_when_not_configured_returns_defaults() {
        let ns = NetworkSettings::default();
        let candidates = ns.to_unix_socket_path_candidates();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0], constants::user::UNIX_SOCKET_PATH.as_path());
        assert_eq!(candidates[1], constants::global::UNIX_SOCKET_PATH.as_path());
    }

    // -------------------------------------------------------
    // to_windows_pipe_name_candidates
    // -------------------------------------------------------
    #[test]
    fn windows_pipe_candidates_when_configured() {
        let ns = NetworkSettings {
            unix_socket: None,
            windows_pipe: Some(String::from("custom-pipe")),
        };
        let candidates = ns.to_windows_pipe_name_candidates();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0], "custom-pipe");
    }

    #[test]
    fn windows_pipe_candidates_when_not_configured_returns_defaults() {
        let ns = NetworkSettings::default();
        let candidates = ns.to_windows_pipe_name_candidates();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0], constants::user::WINDOWS_PIPE_NAME.as_str());
        assert_eq!(candidates[1], constants::global::WINDOWS_PIPE_NAME.as_str());
    }

    // -------------------------------------------------------
    // NetworkSettings serde round-trip
    // -------------------------------------------------------
    #[test]
    fn network_settings_serde_round_trip_with_values() {
        let ns = NetworkSettings {
            unix_socket: Some(PathBuf::from("/tmp/test.sock")),
            windows_pipe: Some(String::from("test-pipe")),
        };
        let json = serde_json::to_string(&ns).unwrap();
        let deserialized: NetworkSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(ns, deserialized);
    }

    #[test]
    fn network_settings_serde_round_trip_empty() {
        let ns = NetworkSettings::default();
        let json = serde_json::to_string(&ns).unwrap();
        let deserialized: NetworkSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(ns, deserialized);
    }

    // -------------------------------------------------------
    // PartialEq
    // -------------------------------------------------------
    #[test]
    fn network_settings_equality() {
        let a = NetworkSettings {
            unix_socket: Some(PathBuf::from("/a")),
            windows_pipe: Some(String::from("a")),
        };
        let b = NetworkSettings {
            unix_socket: Some(PathBuf::from("/a")),
            windows_pipe: Some(String::from("a")),
        };
        let c = NetworkSettings {
            unix_socket: Some(PathBuf::from("/c")),
            windows_pipe: None,
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
