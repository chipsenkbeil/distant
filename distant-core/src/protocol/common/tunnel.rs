use serde::{Deserialize, Serialize};

/// Id for a remote tunnel.
pub type TunnelId = u32;

/// Information about an active tunnel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelInfo {
    /// Unique identifier for the tunnel.
    pub id: TunnelId,
    /// Direction of the tunnel (forward or reverse).
    pub direction: TunnelDirection,
    /// The host the tunnel is connected or bound to.
    pub host: String,
    /// The port the tunnel is connected or bound to.
    pub port: u16,
}

/// Direction of a tunnel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelDirection {
    /// Forward tunnel: client connects to a remote host:port.
    Forward,
    /// Reverse tunnel: server listens on a host:port for incoming connections.
    Reverse,
}

#[cfg(test)]
mod tests {
    use super::*;

    mod tunnel_info {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let info = TunnelInfo {
                id: 42,
                direction: TunnelDirection::Forward,
                host: String::from("localhost"),
                port: 8080,
            };

            let value = serde_json::to_value(info).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "id": 42,
                    "direction": "forward",
                    "host": "localhost",
                    "port": 8080,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "id": 42,
                "direction": "forward",
                "host": "localhost",
                "port": 8080,
            });

            let info: TunnelInfo = serde_json::from_value(value).unwrap();
            assert_eq!(
                info,
                TunnelInfo {
                    id: 42,
                    direction: TunnelDirection::Forward,
                    host: String::from("localhost"),
                    port: 8080,
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let info = TunnelInfo {
                id: 42,
                direction: TunnelDirection::Forward,
                host: String::from("localhost"),
                port: 8080,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&info).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&TunnelInfo {
                id: 42,
                direction: TunnelDirection::Forward,
                host: String::from("localhost"),
                port: 8080,
            })
            .unwrap();

            let info: TunnelInfo = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                info,
                TunnelInfo {
                    id: 42,
                    direction: TunnelDirection::Forward,
                    host: String::from("localhost"),
                    port: 8080,
                }
            );
        }
    }

    mod tunnel_direction {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_forward_to_json() {
            let value = serde_json::to_value(TunnelDirection::Forward).unwrap();
            assert_eq!(value, serde_json::json!("forward"));
        }

        #[test]
        fn should_be_able_to_serialize_reverse_to_json() {
            let value = serde_json::to_value(TunnelDirection::Reverse).unwrap();
            assert_eq!(value, serde_json::json!("reverse"));
        }

        #[test]
        fn should_be_able_to_deserialize_forward_from_json() {
            let dir: TunnelDirection =
                serde_json::from_value(serde_json::json!("forward")).unwrap();
            assert_eq!(dir, TunnelDirection::Forward);
        }

        #[test]
        fn should_be_able_to_deserialize_reverse_from_json() {
            let dir: TunnelDirection =
                serde_json::from_value(serde_json::json!("reverse")).unwrap();
            assert_eq!(dir, TunnelDirection::Reverse);
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&TunnelDirection::Forward).unwrap();
            let _ = rmp_serde::encode::to_vec_named(&TunnelDirection::Reverse).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&TunnelDirection::Forward).unwrap();
            let dir: TunnelDirection = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(dir, TunnelDirection::Forward);

            let buf = rmp_serde::encode::to_vec_named(&TunnelDirection::Reverse).unwrap();
            let dir: TunnelDirection = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(dir, TunnelDirection::Reverse);
        }
    }
}
