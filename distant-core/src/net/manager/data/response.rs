use crate::auth::msg::Authentication;
use crate::protocol::{MountInfo, TunnelDirection};
use serde::{Deserialize, Serialize};

use super::{
    ConnectionInfo, ConnectionList, Event, ManagedTunnelId, ManagerAuthenticationId,
    ManagerChannelId, SemVer,
};
use crate::net::common::{ConnectionId, Destination, UntypedResponse};

/// Detailed information about any managed resource, returned by `Info { id }`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ResourceInfo {
    /// Connection details.
    Connection(ConnectionInfo),
    /// Tunnel details.
    Tunnel(ManagedTunnelInfo),
    /// Mount details.
    Mount(MountInfo),
}

/// Information about a tunnel managed by the manager process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedTunnelInfo {
    pub id: ManagedTunnelId,
    pub connection_id: ConnectionId,
    pub direction: TunnelDirection,
    pub bind_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerResponse {
    /// Acknowledgement that a connection was killed
    Killed,

    /// Indicates that some error occurred during a request
    Error { description: String },

    /// Information about the manager's version.
    Version { version: SemVer },

    /// Confirmation of a server being launched
    Launched {
        /// Updated location of the spawned server
        destination: Destination,
    },

    /// Confirmation of a connection being established
    Connected { id: ConnectionId },

    /// Authentication information being sent to a client
    Authenticate {
        /// Id tied to authentication information in case a response is needed
        id: ManagerAuthenticationId,

        /// Authentication message
        msg: Authentication,
    },

    /// Information about a specific connection
    Info(ConnectionInfo),

    /// List of connections in the form of id -> destination
    List(ConnectionList),

    /// Forward a response back to a specific channel that made a request
    Channel {
        /// Id of the channel
        id: ManagerChannelId,

        /// Untyped response to send through the channel
        response: UntypedResponse<'static>,
    },

    /// Indicates that a channel has been opened
    ChannelOpened {
        /// Id of the channel
        id: ManagerChannelId,
    },

    /// Indicates that a channel has been closed
    ChannelClosed {
        /// Id of the channel
        id: ManagerChannelId,
    },

    /// Confirmation that a managed tunnel was started
    ManagedTunnelStarted { id: ManagedTunnelId, port: u16 },

    /// Acknowledgement that a managed tunnel was closed
    ManagedTunnelClosed,

    /// List of managed tunnels
    ManagedTunnels { tunnels: Vec<ManagedTunnelInfo> },

    /// Confirmation that a mount was created.
    Mounted {
        /// Unique mount identifier.
        id: u32,
        /// Local mount point path.
        mount_point: String,
        /// Backend name.
        backend: String,
    },

    /// Acknowledgement that mounts were removed.
    Unmounted {
        /// IDs that were successfully unmounted.
        ids: Vec<u32>,
    },

    /// List of active mounts.
    Mounts { mounts: Vec<MountInfo> },

    /// Acknowledgement of a `Subscribe` request.
    Subscribed,

    /// Acknowledgement of an `Unsubscribe` request.
    Unsubscribed,

    /// Push notification — only sent on subscribed channels.
    Event {
        /// The event payload.
        event: Event,
    },

    /// Acknowledgement that a manual reconnection was started.
    /// The actual state transition arrives later via an
    /// `Event::ConnectionState` push.
    ReconnectInitiated {
        /// Id of the connection being reconnected.
        id: ConnectionId,
    },
}

impl<T: std::error::Error> From<T> for ManagerResponse {
    fn from(x: T) -> Self {
        Self::Error {
            description: x.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::TunnelDirection;

    fn make_tunnel_info() -> ManagedTunnelInfo {
        ManagedTunnelInfo {
            id: 42,
            connection_id: 7,
            direction: TunnelDirection::Forward,
            bind_port: 8080,
            remote_host: "db-host".to_string(),
            remote_port: 5432,
        }
    }

    #[test]
    fn managed_tunnel_info_should_serialize_and_deserialize_via_json() {
        let info = make_tunnel_info();
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: ManagedTunnelInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn managed_tunnel_info_should_preserve_all_fields_through_json_roundtrip() {
        let info = ManagedTunnelInfo {
            id: 99,
            connection_id: 12,
            direction: TunnelDirection::Reverse,
            bind_port: 0,
            remote_host: "[::1]".to_string(),
            remote_port: 443,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: ManagedTunnelInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, 99);
        assert_eq!(deserialized.connection_id, 12);
        assert_eq!(deserialized.direction, TunnelDirection::Reverse);
        assert_eq!(deserialized.bind_port, 0);
        assert_eq!(deserialized.remote_host, "[::1]");
        assert_eq!(deserialized.remote_port, 443);
    }

    #[test]
    fn managed_tunnel_info_should_serialize_direction_as_snake_case() {
        let forward = make_tunnel_info();
        let json = serde_json::to_string(&forward).unwrap();
        assert!(
            json.contains("\"forward\""),
            "Expected 'forward' in JSON: {json}"
        );

        let reverse = ManagedTunnelInfo {
            direction: TunnelDirection::Reverse,
            ..make_tunnel_info()
        };
        let json = serde_json::to_string(&reverse).unwrap();
        assert!(
            json.contains("\"reverse\""),
            "Expected 'reverse' in JSON: {json}"
        );
    }

    #[test]
    fn managed_tunnel_info_clone_should_produce_equal_value() {
        let info = make_tunnel_info();
        let cloned = info.clone();
        assert_eq!(info, cloned);
    }

    #[test]
    fn managed_tunnel_info_should_not_equal_when_fields_differ() {
        let info = make_tunnel_info();
        let different = ManagedTunnelInfo {
            id: 999,
            ..info.clone()
        };
        assert_ne!(info, different);
    }

    #[test]
    fn managed_tunnel_started_should_serialize_and_deserialize_via_json() {
        let response = ManagerResponse::ManagedTunnelStarted { id: 5, port: 9090 };
        let json = serde_json::to_string(&response).unwrap();

        assert!(
            json.contains("\"managed_tunnel_started\""),
            "Expected snake_case variant tag in JSON: {json}"
        );

        let deserialized: ManagerResponse = serde_json::from_str(&json).unwrap();
        match deserialized {
            ManagerResponse::ManagedTunnelStarted { id, port } => {
                assert_eq!(id, 5);
                assert_eq!(port, 9090);
            }
            other => panic!("Expected ManagedTunnelStarted, got {other:?}"),
        }
    }

    #[test]
    fn managed_tunnel_closed_should_serialize_and_deserialize_via_json() {
        let response = ManagerResponse::ManagedTunnelClosed;
        let json = serde_json::to_string(&response).unwrap();

        assert!(
            json.contains("\"managed_tunnel_closed\""),
            "Expected snake_case variant tag in JSON: {json}"
        );

        let deserialized: ManagerResponse = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(deserialized, ManagerResponse::ManagedTunnelClosed),
            "Expected ManagedTunnelClosed, got {deserialized:?}"
        );
    }

    #[test]
    fn managed_tunnels_should_serialize_and_deserialize_via_json() {
        let tunnels = vec![
            ManagedTunnelInfo {
                id: 1,
                connection_id: 10,
                direction: TunnelDirection::Forward,
                bind_port: 8080,
                remote_host: "host-a".to_string(),
                remote_port: 80,
            },
            ManagedTunnelInfo {
                id: 2,
                connection_id: 10,
                direction: TunnelDirection::Reverse,
                bind_port: 3306,
                remote_host: "host-b".to_string(),
                remote_port: 3306,
            },
        ];
        let response = ManagerResponse::ManagedTunnels {
            tunnels: tunnels.clone(),
        };
        let json = serde_json::to_string(&response).unwrap();

        assert!(
            json.contains("\"managed_tunnels\""),
            "Expected snake_case variant tag in JSON: {json}"
        );

        let deserialized: ManagerResponse = serde_json::from_str(&json).unwrap();
        match deserialized {
            ManagerResponse::ManagedTunnels {
                tunnels: deserialized_tunnels,
            } => {
                assert_eq!(deserialized_tunnels, tunnels);
            }
            other => panic!("Expected ManagedTunnels, got {other:?}"),
        }
    }

    #[test]
    fn managed_tunnels_should_serialize_empty_list_via_json() {
        let response = ManagerResponse::ManagedTunnels {
            tunnels: Vec::new(),
        };
        let json = serde_json::to_string(&response).unwrap();
        let deserialized: ManagerResponse = serde_json::from_str(&json).unwrap();
        match deserialized {
            ManagerResponse::ManagedTunnels { tunnels } => {
                assert!(tunnels.is_empty(), "Expected empty tunnels vec");
            }
            other => panic!("Expected ManagedTunnels, got {other:?}"),
        }
    }

    #[test]
    fn from_error_should_create_error_response_with_description() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let response = ManagerResponse::from(io_err);
        match response {
            ManagerResponse::Error { description } => {
                assert_eq!(description, "file missing");
            }
            other => panic!("Expected Error variant, got {other:?}"),
        }
    }

    #[test]
    fn managed_tunnel_started_should_reject_unknown_fields() {
        let json = r#"{"type":"managed_tunnel_started","id":1,"port":8080,"extra":"bad"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(
            result.is_err(),
            "Expected deserialization to fail on unknown field"
        );
    }

    #[test]
    fn managed_tunnels_should_reject_unknown_fields() {
        let json = r#"{"type":"managed_tunnels","tunnels":[],"extra":"bad"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(
            result.is_err(),
            "Expected deserialization to fail on unknown field"
        );
    }

    #[test]
    fn subscribed_should_serialize_with_snake_case_tag() {
        let response = ManagerResponse::Subscribed;
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, "{\"type\":\"subscribed\"}");
        let deserialized: ManagerResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, ManagerResponse::Subscribed));
    }

    #[test]
    fn unsubscribed_should_serialize_with_snake_case_tag() {
        let response = ManagerResponse::Unsubscribed;
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, "{\"type\":\"unsubscribed\"}");
        let deserialized: ManagerResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, ManagerResponse::Unsubscribed));
    }

    #[test]
    fn reconnect_initiated_should_round_trip_via_json() {
        let response = ManagerResponse::ReconnectInitiated { id: 7 };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"reconnect_initiated\""), "got {json}");
        assert!(json.contains("\"id\":7"), "got {json}");
        let deserialized: ManagerResponse = serde_json::from_str(&json).unwrap();
        match deserialized {
            ManagerResponse::ReconnectInitiated { id } => assert_eq!(id, 7),
            other => panic!("Expected ReconnectInitiated, got {other:?}"),
        }
    }

    #[test]
    fn event_response_should_wrap_inner_event_via_json() {
        use crate::net::client::ConnectionState;
        let response = ManagerResponse::Event {
            event: Event::ConnectionState {
                id: 11,
                state: ConnectionState::Reconnecting,
            },
        };
        let value: serde_json::Value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["type"], "event");
        assert_eq!(value["event"]["type"], "connection_state");
        assert_eq!(value["event"]["id"], 11);
        assert_eq!(value["event"]["state"], "reconnecting");

        let json = serde_json::to_string(&response).unwrap();
        let decoded: ManagerResponse = serde_json::from_str(&json).unwrap();
        match decoded {
            ManagerResponse::Event {
                event: Event::ConnectionState { id, state },
            } => {
                assert_eq!(id, 11);
                assert_eq!(state, ConnectionState::Reconnecting);
            }
            other => panic!("Expected Event(ConnectionState), got {other:?}"),
        }
    }
}
