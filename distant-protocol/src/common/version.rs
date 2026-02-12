use serde::{Deserialize, Serialize};

use crate::semver;

/// Represents version information.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Server version.
    pub server_version: semver::Version,

    /// Protocol version.
    pub protocol_version: semver::Version,

    /// Additional features available.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl Version {
    /// Supports executing processes.
    pub const CAP_EXEC: &'static str = "exec";
    /// Supports reading and writing via filesystem IO.
    pub const CAP_FS_IO: &'static str = "fs_io";
    /// Supports modifying permissions of filesystem.
    pub const CAP_FS_PERM: &'static str = "fs_perm";
    /// Supports searching filesystem.
    pub const CAP_FS_SEARCH: &'static str = "fs_search";
    /// Supports watching filesystem for changes.
    pub const CAP_FS_WATCH: &'static str = "fs_watch";
    // /// Supports TCP tunneling.
    // pub const CAP_TCP_TUNNEL: &'static str = "tcp_tunnel";
    //
    // /// Supports TCP reverse tunneling.
    // pub const CAP_TCP_REV_TUNNEL: &'static str = "tcp_rev_tunnel";

    /// Supports retrieving system information.
    pub const CAP_SYS_INFO: &'static str = "sys_info";

    pub const fn capabilities() -> &'static [&'static str] {
        &[
            Self::CAP_EXEC,
            Self::CAP_FS_IO,
            Self::CAP_FS_PERM,
            Self::CAP_FS_SEARCH,
            Self::CAP_FS_WATCH,
            /* Self::CAP_TCP_TUNNEL,
            Self::CAP_TCP_REV_TUNNEL, */
            Self::CAP_SYS_INFO,
        ]
    }
}

#[cfg(test)]
mod tests {
    use semver::Version as SemVer;

    use super::*;

    #[test]
    fn should_be_able_to_serialize_to_json() {
        let version = Version {
            server_version: "123.456.789-rc+build".parse().unwrap(),
            protocol_version: SemVer::new(1, 2, 3),
            capabilities: vec![String::from("cap")],
        };

        let value = serde_json::to_value(version).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "server_version": "123.456.789-rc+build",
                "protocol_version": "1.2.3",
                "capabilities": ["cap"]
            })
        );
    }

    #[test]
    fn should_be_able_to_deserialize_from_json() {
        let value = serde_json::json!({
            "server_version": "123.456.789-rc+build",
            "protocol_version": "1.2.3",
            "capabilities": ["cap"]
        });

        let version: Version = serde_json::from_value(value).unwrap();
        assert_eq!(
            version,
            Version {
                server_version: "123.456.789-rc+build".parse().unwrap(),
                protocol_version: SemVer::new(1, 2, 3),
                capabilities: vec![String::from("cap")],
            }
        );
    }

    #[test]
    fn should_be_able_to_serialize_to_msgpack() {
        let version = Version {
            server_version: "123.456.789-rc+build".parse().unwrap(),
            protocol_version: SemVer::new(1, 2, 3),
            capabilities: vec![String::from("cap")],
        };

        // NOTE: We don't actually check the output here because it's an implementation detail
        // and could change as we change how serialization is done. This is merely to verify
        // that we can serialize since there are times when serde fails to serialize at
        // runtime.
        let _ = rmp_serde::encode::to_vec_named(&version).unwrap();
    }

    #[test]
    fn should_be_able_to_deserialize_from_msgpack() {
        // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
        // verify that we are not corrupting or causing issues when serializing on a
        // client/server and then trying to deserialize on the other side. This has happened
        // enough times with minor changes that we need tests to verify.
        let buf = rmp_serde::encode::to_vec_named(&Version {
            server_version: "123.456.789-rc+build".parse().unwrap(),
            protocol_version: SemVer::new(1, 2, 3),
            capabilities: vec![String::from("cap")],
        })
        .unwrap();

        let version: Version = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(
            version,
            Version {
                server_version: "123.456.789-rc+build".parse().unwrap(),
                protocol_version: SemVer::new(1, 2, 3),
                capabilities: vec![String::from("cap")],
            }
        );
    }
}
