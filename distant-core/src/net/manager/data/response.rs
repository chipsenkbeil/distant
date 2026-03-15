use crate::auth::msg::Authentication;
use serde::{Deserialize, Serialize};

use super::{ConnectionInfo, ConnectionList, ManagerAuthenticationId, ManagerChannelId, SemVer};
use crate::net::client::ConnectionState;
use crate::net::common::{ConnectionId, Destination, UntypedResponse};

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

    /// Unsolicited notification of a connection state change.
    /// Sent to clients that have subscribed via `SubscribeConnectionEvents`.
    ConnectionStateChanged {
        /// Id of the connection whose state changed
        id: ConnectionId,
        /// New connection state
        state: ConnectionState,
    },

    /// Confirmation that connection event subscription was established.
    SubscribedConnectionEvents,

    /// Confirmation that a reconnection attempt has been initiated.
    ReconnectInitiated {
        /// Id of the connection
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
    use test_log::test;

    use super::*;

    // ---------------------------------------------------------------
    // ConnectionStateChanged serde round-trip
    // ---------------------------------------------------------------

    #[test]
    fn connection_state_changed_serde_round_trip() {
        let resp = ManagerResponse::ConnectionStateChanged {
            id: 7,
            state: ConnectionState::Disconnected,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["type"], "connection_state_changed");
        assert_eq!(val["id"], 7);
        assert_eq!(val["state"], "disconnected");

        let restored: ManagerResponse = serde_json::from_str(&json).unwrap();
        match restored {
            ManagerResponse::ConnectionStateChanged { id, state } => {
                assert_eq!(id, 7);
                assert_eq!(state, ConnectionState::Disconnected);
            }
            other => panic!("Expected ConnectionStateChanged, got {other:?}"),
        }
    }

    #[test]
    fn connection_state_changed_with_reconnecting_state() {
        let resp = ManagerResponse::ConnectionStateChanged {
            id: 15,
            state: ConnectionState::Reconnecting,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["state"], "reconnecting");

        let restored: ManagerResponse = serde_json::from_str(&json).unwrap();
        match restored {
            ManagerResponse::ConnectionStateChanged { id, state } => {
                assert_eq!(id, 15);
                assert_eq!(state, ConnectionState::Reconnecting);
            }
            other => panic!("Expected ConnectionStateChanged, got {other:?}"),
        }
    }

    #[test]
    fn connection_state_changed_with_connected_state() {
        let resp = ManagerResponse::ConnectionStateChanged {
            id: 0,
            state: ConnectionState::Connected,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["state"], "connected");

        let restored: ManagerResponse = serde_json::from_str(&json).unwrap();
        match restored {
            ManagerResponse::ConnectionStateChanged { id, state } => {
                assert_eq!(id, 0);
                assert_eq!(state, ConnectionState::Connected);
            }
            other => panic!("Expected ConnectionStateChanged, got {other:?}"),
        }
    }

    #[test]
    fn connection_state_changed_from_raw_json() {
        let json = r#"{"type":"connection_state_changed","id":42,"state":"disconnected"}"#;
        let resp: ManagerResponse = serde_json::from_str(json).unwrap();
        match resp {
            ManagerResponse::ConnectionStateChanged { id, state } => {
                assert_eq!(id, 42);
                assert_eq!(state, ConnectionState::Disconnected);
            }
            other => panic!("Expected ConnectionStateChanged, got {other:?}"),
        }
    }

    #[test]
    fn connection_state_changed_should_reject_missing_fields() {
        let json = r#"{"type":"connection_state_changed","id":1}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());

        let json = r#"{"type":"connection_state_changed","state":"connected"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    #[test]
    fn connection_state_changed_should_reject_invalid_state() {
        let json = r#"{"type":"connection_state_changed","id":1,"state":"unknown"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // SubscribedConnectionEvents serde round-trip
    // ---------------------------------------------------------------

    #[test]
    fn subscribed_connection_events_serde_round_trip() {
        let resp = ManagerResponse::SubscribedConnectionEvents;
        let json = serde_json::to_string(&resp).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["type"], "subscribed_connection_events");

        let restored: ManagerResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            restored,
            ManagerResponse::SubscribedConnectionEvents
        ));
    }

    #[test]
    fn subscribed_connection_events_from_raw_json() {
        let json = r#"{"type":"subscribed_connection_events"}"#;
        let resp: ManagerResponse = serde_json::from_str(json).unwrap();
        assert!(matches!(resp, ManagerResponse::SubscribedConnectionEvents));
    }

    #[test]
    fn subscribed_connection_events_ignores_extra_fields_due_to_serde_unit_variant_limitation() {
        // NOTE: serde's `deny_unknown_fields` with `#[serde(tag = "type")]` does NOT
        // reject extra fields on unit variants -- the tag is consumed and remaining
        // fields are silently ignored. This test documents this known serde behavior.
        let json = r#"{"type":"subscribed_connection_events","extra":1}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(matches!(
            result.unwrap(),
            ManagerResponse::SubscribedConnectionEvents
        ));
    }

    // ---------------------------------------------------------------
    // ReconnectInitiated serde round-trip
    // ---------------------------------------------------------------

    #[test]
    fn reconnect_initiated_serde_round_trip() {
        let resp = ManagerResponse::ReconnectInitiated { id: 55 };
        let json = serde_json::to_string(&resp).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["type"], "reconnect_initiated");
        assert_eq!(val["id"], 55);

        let restored: ManagerResponse = serde_json::from_str(&json).unwrap();
        match restored {
            ManagerResponse::ReconnectInitiated { id } => assert_eq!(id, 55),
            other => panic!("Expected ReconnectInitiated, got {other:?}"),
        }
    }

    #[test]
    fn reconnect_initiated_from_raw_json() {
        let json = r#"{"type":"reconnect_initiated","id":100}"#;
        let resp: ManagerResponse = serde_json::from_str(json).unwrap();
        match resp {
            ManagerResponse::ReconnectInitiated { id } => assert_eq!(id, 100),
            other => panic!("Expected ReconnectInitiated, got {other:?}"),
        }
    }

    #[test]
    fn reconnect_initiated_should_reject_missing_id() {
        let json = r#"{"type":"reconnect_initiated"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reconnect_initiated_should_reject_extra_fields() {
        let json = r#"{"type":"reconnect_initiated","id":1,"extra":"nope"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // ConnectionStateChanged additional edge cases
    // ---------------------------------------------------------------

    #[test]
    fn connection_state_changed_should_reject_extra_fields() {
        let json =
            r#"{"type":"connection_state_changed","id":1,"state":"connected","extra":"nope"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    #[test]
    fn connection_state_changed_should_reject_capitalized_state() {
        // ConnectionState uses snake_case, so "Connected" should fail
        let json = r#"{"type":"connection_state_changed","id":1,"state":"Connected"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reconnect_initiated_should_reject_wrong_id_type() {
        let json = r#"{"type":"reconnect_initiated","id":"not_a_number"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    #[test]
    fn connection_state_changed_should_reject_wrong_id_type() {
        let json = r#"{"type":"connection_state_changed","id":"abc","state":"connected"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // Wrong-case type tag rejection
    // ---------------------------------------------------------------

    #[test]
    fn connection_state_changed_should_reject_wrong_case_type() {
        let json = r#"{"type":"ConnectionStateChanged","id":1,"state":"connected"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    #[test]
    fn subscribed_connection_events_should_reject_wrong_case_type() {
        let json = r#"{"type":"SubscribedConnectionEvents"}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    #[test]
    fn reconnect_initiated_should_reject_wrong_case_type() {
        let json = r#"{"type":"ReconnectInitiated","id":1}"#;
        let result = serde_json::from_str::<ManagerResponse>(json);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // Boundary value round-trips
    // ---------------------------------------------------------------

    #[test]
    fn connection_state_changed_should_preserve_boundary_id_values() {
        for id in [0u32, 1, u32::MAX] {
            let resp = ManagerResponse::ConnectionStateChanged {
                id,
                state: ConnectionState::Connected,
            };
            let json = serde_json::to_string(&resp).unwrap();
            let restored: ManagerResponse = serde_json::from_str(&json).unwrap();
            match restored {
                ManagerResponse::ConnectionStateChanged {
                    id: restored_id,
                    state,
                } => {
                    assert_eq!(restored_id, id, "id {id} should survive round-trip");
                    assert_eq!(state, ConnectionState::Connected);
                }
                other => panic!("Expected ConnectionStateChanged, got {other:?}"),
            }
        }
    }

    #[test]
    fn reconnect_initiated_should_preserve_boundary_id_values() {
        for id in [0u32, 1, u32::MAX] {
            let resp = ManagerResponse::ReconnectInitiated { id };
            let json = serde_json::to_string(&resp).unwrap();
            let restored: ManagerResponse = serde_json::from_str(&json).unwrap();
            match restored {
                ManagerResponse::ReconnectInitiated { id: restored_id } => {
                    assert_eq!(restored_id, id, "id {id} should survive round-trip");
                }
                other => panic!("Expected ReconnectInitiated, got {other:?}"),
            }
        }
    }
}
