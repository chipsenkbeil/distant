use serde::{Deserialize, Serialize};

use crate::common::{Capabilities, SemVer};

/// Represents version information.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// General version of server (arbitrary format)
    pub server_version: String,

    /// Protocol version
    pub protocol_version: SemVer,

    /// Capabilities of the server
    pub capabilities: Capabilities,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Capability;

    #[test]
    fn should_be_able_to_serialize_to_json() {
        let version = Version {
            server_version: String::from("some version"),
            protocol_version: (1, 2, 3),
            capabilities: [Capability {
                kind: String::from("some kind"),
                description: String::from("some description"),
            }]
            .into_iter()
            .collect(),
        };

        let value = serde_json::to_value(version).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "server_version": "some version",
                "protocol_version": [1, 2, 3],
                "capabilities": [{
                    "kind": "some kind",
                    "description": "some description",
                }]
            })
        );
    }

    #[test]
    fn should_be_able_to_deserialize_from_json() {
        let value = serde_json::json!({
            "server_version": "some version",
            "protocol_version": [1, 2, 3],
            "capabilities": [{
                "kind": "some kind",
                "description": "some description",
            }]
        });

        let version: Version = serde_json::from_value(value).unwrap();
        assert_eq!(
            version,
            Version {
                server_version: String::from("some version"),
                protocol_version: (1, 2, 3),
                capabilities: [Capability {
                    kind: String::from("some kind"),
                    description: String::from("some description"),
                }]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn should_be_able_to_serialize_to_msgpack() {
        let version = Version {
            server_version: String::from("some version"),
            protocol_version: (1, 2, 3),
            capabilities: [Capability {
                kind: String::from("some kind"),
                description: String::from("some description"),
            }]
            .into_iter()
            .collect(),
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
            server_version: String::from("some version"),
            protocol_version: (1, 2, 3),
            capabilities: [Capability {
                kind: String::from("some kind"),
                description: String::from("some description"),
            }]
            .into_iter()
            .collect(),
        })
        .unwrap();

        let version: Version = rmp_serde::decode::from_slice(&buf).unwrap();
        assert_eq!(
            version,
            Version {
                server_version: String::from("some version"),
                protocol_version: (1, 2, 3),
                capabilities: [Capability {
                    kind: String::from("some kind"),
                    description: String::from("some description"),
                }]
                .into_iter()
                .collect(),
            }
        );
    }
}
