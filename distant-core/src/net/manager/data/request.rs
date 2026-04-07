use crate::auth::msg::AuthenticationResponse;
use serde::{Deserialize, Serialize};

use super::{EventTopic, ManagedTunnelId, ManagerAuthenticationId, ManagerChannelId};
use crate::net::common::{ConnectionId, Map, UntypedRequest};
use crate::protocol::{MountConfig, ResourceKind};

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerRequest {
    /// Retrieve information about the manager's version.
    Version,

    /// Launch a server using the manager
    Launch {
        /// Raw destination string (e.g. `"docker://ubuntu:22.04"` or `"ssh://host:22"`).
        /// Parsing is deferred to the plugin matched by scheme.
        destination: String,

        /// Additional options specific to the connection
        options: Map,
    },

    /// Initiate a connection through the manager
    Connect {
        /// Raw destination string. Parsing is deferred to the plugin matched by scheme.
        destination: String,

        /// Additional options specific to the connection
        options: Map,
    },

    /// Submit some authentication message for the manager to use with an active connection
    Authenticate {
        /// Id of the authentication request that is being responded to
        id: ManagerAuthenticationId,

        /// Response being sent to some active connection
        msg: AuthenticationResponse,
    },

    /// Opens a channel for communication with an already-connected server
    OpenChannel {
        /// Id of the connection
        id: ConnectionId,
    },

    /// Sends data through channel
    Channel {
        /// Id of the channel
        id: ManagerChannelId,

        /// Untyped request to send through the channel
        request: UntypedRequest<'static>,
    },

    /// Closes an open channel
    CloseChannel {
        /// Id of the channel to close
        id: ManagerChannelId,
    },

    /// Retrieve information about a specific connection
    Info { id: ConnectionId },

    /// Kill a specific connection
    Kill { id: ConnectionId },

    /// Retrieve list of managed resources.
    ///
    /// When `resources` is empty, all resource types are returned.
    List {
        #[serde(default)]
        resources: Vec<ResourceKind>,
    },

    /// Start a forward tunnel (local listener -> remote target) in the manager
    ForwardTunnel {
        connection_id: ConnectionId,
        bind_port: u16,
        remote_host: String,
        remote_port: u16,
    },

    /// Start a reverse tunnel (remote listener -> local target) in the manager
    ReverseTunnel {
        connection_id: ConnectionId,
        remote_port: u16,
        local_host: String,
        local_port: u16,
    },

    /// Close a managed tunnel by ID
    CloseManagedTunnel { id: ManagedTunnelId },

    /// List all managed tunnels
    ListManagedTunnels,

    /// Mount a remote filesystem via a mount plugin.
    Mount {
        /// Connection to use for the mount.
        connection_id: ConnectionId,

        /// Backend name (e.g., "nfs", "fuse", "macos-file-provider", "windows-cloud-files").
        backend: String,

        /// Mount configuration.
        config: MountConfig,
    },

    /// Unmount one or more mounted filesystems.
    Unmount {
        /// Mount IDs to unmount.
        ids: Vec<u32>,
    },

    /// Subscribe to event notifications on this channel.
    ///
    /// After the manager replies with `Subscribed`, it begins
    /// pushing `Event(_)` responses for every event whose topic is
    /// in `topics`. The subscription stays open until either the
    /// channel closes or an `Unsubscribe` request arrives.
    Subscribe {
        /// Topics to subscribe to. Use `[EventTopic::All]` to receive
        /// every event variant.
        topics: Vec<EventTopic>,
    },

    /// Cancel a previous subscription on this channel.
    Unsubscribe,

    /// Manually trigger reconnection of a managed connection.
    Reconnect {
        /// Id of the connection to reconnect.
        id: ConnectionId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_tunnel_should_serialize_and_deserialize_via_json() {
        let request = ManagerRequest::ForwardTunnel {
            connection_id: 5,
            bind_port: 8080,
            remote_host: "db-host".to_string(),
            remote_port: 5432,
        };
        let json = serde_json::to_string(&request).unwrap();

        assert!(
            json.contains("\"forward_tunnel\""),
            "Expected snake_case variant tag in JSON: {json}"
        );

        let deserialized: ManagerRequest = serde_json::from_str(&json).unwrap();
        match deserialized {
            ManagerRequest::ForwardTunnel {
                connection_id,
                bind_port,
                remote_host,
                remote_port,
            } => {
                assert_eq!(connection_id, 5);
                assert_eq!(bind_port, 8080);
                assert_eq!(remote_host, "db-host");
                assert_eq!(remote_port, 5432);
            }
            other => panic!("Expected ForwardTunnel, got {other:?}"),
        }
    }

    #[test]
    fn reverse_tunnel_should_serialize_and_deserialize_via_json() {
        let request = ManagerRequest::ReverseTunnel {
            connection_id: 3,
            remote_port: 9000,
            local_host: "localhost".to_string(),
            local_port: 3000,
        };
        let json = serde_json::to_string(&request).unwrap();

        assert!(
            json.contains("\"reverse_tunnel\""),
            "Expected snake_case variant tag in JSON: {json}"
        );

        let deserialized: ManagerRequest = serde_json::from_str(&json).unwrap();
        match deserialized {
            ManagerRequest::ReverseTunnel {
                connection_id,
                remote_port,
                local_host,
                local_port,
            } => {
                assert_eq!(connection_id, 3);
                assert_eq!(remote_port, 9000);
                assert_eq!(local_host, "localhost");
                assert_eq!(local_port, 3000);
            }
            other => panic!("Expected ReverseTunnel, got {other:?}"),
        }
    }

    #[test]
    fn close_managed_tunnel_should_serialize_and_deserialize_via_json() {
        let request = ManagerRequest::CloseManagedTunnel { id: 42 };
        let json = serde_json::to_string(&request).unwrap();

        assert!(
            json.contains("\"close_managed_tunnel\""),
            "Expected snake_case variant tag in JSON: {json}"
        );

        let deserialized: ManagerRequest = serde_json::from_str(&json).unwrap();
        match deserialized {
            ManagerRequest::CloseManagedTunnel { id } => {
                assert_eq!(id, 42);
            }
            other => panic!("Expected CloseManagedTunnel, got {other:?}"),
        }
    }

    #[test]
    fn list_managed_tunnels_should_serialize_and_deserialize_via_json() {
        let request = ManagerRequest::ListManagedTunnels;
        let json = serde_json::to_string(&request).unwrap();

        assert!(
            json.contains("\"list_managed_tunnels\""),
            "Expected snake_case variant tag in JSON: {json}"
        );

        let deserialized: ManagerRequest = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(deserialized, ManagerRequest::ListManagedTunnels),
            "Expected ListManagedTunnels, got {deserialized:?}"
        );
    }

    #[test]
    fn forward_tunnel_should_reject_unknown_fields() {
        let json = r#"{"type":"forward_tunnel","connection_id":1,"bind_port":80,"remote_host":"h","remote_port":80,"extra":"bad"}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(
            result.is_err(),
            "Expected deserialization to fail on unknown field"
        );
    }

    #[test]
    fn reverse_tunnel_should_reject_unknown_fields() {
        let json = r#"{"type":"reverse_tunnel","connection_id":1,"remote_port":80,"local_host":"h","local_port":80,"extra":"bad"}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(
            result.is_err(),
            "Expected deserialization to fail on unknown field"
        );
    }

    #[test]
    fn close_managed_tunnel_should_reject_unknown_fields() {
        let json = r#"{"type":"close_managed_tunnel","id":1,"extra":"bad"}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(
            result.is_err(),
            "Expected deserialization to fail on unknown field"
        );
    }

    #[test]
    fn subscribe_should_round_trip_with_topics() {
        let request = ManagerRequest::Subscribe {
            topics: vec![EventTopic::Connection, EventTopic::Mount],
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"subscribe\""), "got {json}");
        assert!(json.contains("\"connection\""), "got {json}");
        let deserialized: ManagerRequest = serde_json::from_str(&json).unwrap();
        match deserialized {
            ManagerRequest::Subscribe { topics } => {
                assert_eq!(topics, vec![EventTopic::Connection, EventTopic::Mount]);
            }
            other => panic!("Expected Subscribe, got {other:?}"),
        }
    }

    #[test]
    fn subscribe_should_accept_empty_topics_list() {
        let json = r#"{"type":"subscribe","topics":[]}"#;
        let request: ManagerRequest = serde_json::from_str(json).unwrap();
        match request {
            ManagerRequest::Subscribe { topics } => assert!(topics.is_empty()),
            other => panic!("Expected Subscribe, got {other:?}"),
        }
    }

    #[test]
    fn unsubscribe_should_round_trip_via_json() {
        let request = ManagerRequest::Unsubscribe;
        let json = serde_json::to_string(&request).unwrap();
        assert_eq!(json, "{\"type\":\"unsubscribe\"}");
        let deserialized: ManagerRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, ManagerRequest::Unsubscribe));
    }

    #[test]
    fn reconnect_should_round_trip_via_json() {
        let request = ManagerRequest::Reconnect { id: 99 };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"reconnect\""), "got {json}");
        assert!(json.contains("\"id\":99"), "got {json}");
        let deserialized: ManagerRequest = serde_json::from_str(&json).unwrap();
        match deserialized {
            ManagerRequest::Reconnect { id } => assert_eq!(id, 99),
            other => panic!("Expected Reconnect, got {other:?}"),
        }
    }

    #[test]
    fn reconnect_should_reject_unknown_fields() {
        let json = r#"{"type":"reconnect","id":1,"extra":"field"}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(
            result.is_err(),
            "Expected deserialization to fail on unknown field"
        );
    }
}
