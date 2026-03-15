use crate::auth::msg::AuthenticationResponse;
use serde::{Deserialize, Serialize};

use super::{ManagerAuthenticationId, ManagerChannelId};
use crate::net::common::{ConnectionId, Map, UntypedRequest};

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

    /// Retrieve list of connections being managed
    List,

    /// Subscribe to connection state change events.
    /// After subscribing, the client receives unsolicited `ConnectionStateChanged` responses.
    SubscribeConnectionEvents,

    /// Request reconnection of a specific connection.
    Reconnect {
        /// Id of the connection to reconnect
        id: ConnectionId,
    },
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn subscribe_connection_events_serde_round_trip() {
        let req = ManagerRequest::SubscribeConnectionEvents;
        let json = serde_json::to_string(&req).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["type"], "subscribe_connection_events");

        let restored: ManagerRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            restored,
            ManagerRequest::SubscribeConnectionEvents
        ));
    }

    #[test]
    fn subscribe_connection_events_from_raw_json() {
        let json = r#"{"type":"subscribe_connection_events"}"#;
        let req: ManagerRequest = serde_json::from_str(json).unwrap();
        assert!(matches!(req, ManagerRequest::SubscribeConnectionEvents));
    }

    #[test]
    fn reconnect_serde_round_trip() {
        let req = ManagerRequest::Reconnect { id: 42 };
        let json = serde_json::to_string(&req).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["type"], "reconnect");
        assert_eq!(val["id"], 42);

        let restored: ManagerRequest = serde_json::from_str(&json).unwrap();
        match restored {
            ManagerRequest::Reconnect { id } => assert_eq!(id, 42),
            other => panic!("Expected Reconnect, got {other:?}"),
        }
    }

    #[test]
    fn reconnect_from_raw_json() {
        let json = r#"{"type":"reconnect","id":99}"#;
        let req: ManagerRequest = serde_json::from_str(json).unwrap();
        match req {
            ManagerRequest::Reconnect { id } => assert_eq!(id, 99),
            other => panic!("Expected Reconnect, got {other:?}"),
        }
    }

    #[test]
    fn reconnect_should_reject_missing_id() {
        let json = r#"{"type":"reconnect"}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn subscribe_connection_events_ignores_extra_fields_due_to_serde_unit_variant_limitation() {
        // NOTE: serde's `deny_unknown_fields` with `#[serde(tag = "type")]` does NOT
        // reject extra fields on unit variants -- the tag is consumed and remaining
        // fields are silently ignored. This test documents this known serde behavior.
        let json = r#"{"type":"subscribe_connection_events","extra":true}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(matches!(
            result.unwrap(),
            ManagerRequest::SubscribeConnectionEvents
        ));
    }

    #[test]
    fn reconnect_should_reject_extra_fields() {
        let json = r#"{"type":"reconnect","id":1,"extra":"field"}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reconnect_should_reject_wrong_id_type() {
        // String instead of number
        let json = r#"{"type":"reconnect","id":"not_a_number"}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn subscribe_connection_events_should_reject_wrong_case_type() {
        // PascalCase instead of snake_case
        let json = r#"{"type":"SubscribeConnectionEvents"}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reconnect_should_reject_wrong_case_type() {
        let json = r#"{"type":"Reconnect","id":1}"#;
        let result = serde_json::from_str::<ManagerRequest>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reconnect_should_preserve_id_value_through_round_trip() {
        // Test boundary values for ConnectionId (u32)
        for id in [0u32, 1, u32::MAX] {
            let req = ManagerRequest::Reconnect { id };
            let json = serde_json::to_string(&req).unwrap();
            let restored: ManagerRequest = serde_json::from_str(&json).unwrap();
            match restored {
                ManagerRequest::Reconnect { id: restored_id } => {
                    assert_eq!(restored_id, id, "id {id} should survive round-trip")
                }
                other => panic!("Expected Reconnect, got {other:?}"),
            }
        }
    }
}
