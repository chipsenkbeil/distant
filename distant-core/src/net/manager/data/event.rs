//! Generic event types broadcast to subscribed manager clients.
//!
//! The event bus allows the manager to push notifications to clients
//! that have subscribed via [`ManagerRequest::Subscribe`]. Each event
//! variant carries the minimum payload needed for callers to act on
//! the change without re-querying the manager.
//!
//! Add new variants here rather than overloading existing ones; the
//! request/response protocol gains new event kinds without needing
//! new request types.

use serde::{Deserialize, Serialize};

use crate::net::client::ConnectionState;
use crate::net::common::ConnectionId;

/// A topic that subscribers filter on. [`Self::All`] matches every
/// existing and future event variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventTopic {
    /// Subscribe to every event topic.
    All,
    /// Connection lifecycle events ([`Event::ConnectionState`]).
    Connection,
    /// Mount lifecycle events.
    ///
    /// Currently no event variants belong to this topic; the variant
    /// is reserved so future additions don't require a protocol bump.
    Mount,
}

/// A push notification delivered through a subscription.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A managed connection's state changed.
    ConnectionState {
        /// Id of the connection whose state changed.
        id: ConnectionId,
        /// New connection state.
        state: ConnectionState,
    },
}

impl Event {
    /// The topic this event belongs to. Used by the subscription
    /// dispatcher to filter events for clients that asked for a
    /// specific topic.
    pub fn topic(&self) -> EventTopic {
        match self {
            Self::ConnectionState { .. } => EventTopic::Connection,
        }
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn connection_state_event_should_serialize_with_type_tag() {
        let event = Event::ConnectionState {
            id: 7,
            state: ConnectionState::Reconnecting,
        };
        let value: serde_json::Value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], "connection_state");
        assert_eq!(value["id"], 7);
        assert_eq!(value["state"], "reconnecting");
    }

    #[test]
    fn connection_state_event_should_round_trip_through_json() {
        let original = Event::ConnectionState {
            id: 42,
            state: ConnectionState::Disconnected,
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn connection_state_event_topic_is_connection() {
        let event = Event::ConnectionState {
            id: 1,
            state: ConnectionState::Connected,
        };
        assert_eq!(event.topic(), EventTopic::Connection);
    }

    #[test]
    fn event_topic_should_round_trip_through_json() {
        for topic in [EventTopic::All, EventTopic::Connection, EventTopic::Mount] {
            let json = serde_json::to_string(&topic).unwrap();
            let decoded: EventTopic = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, topic);
        }
    }

    #[test]
    fn event_topic_serializes_with_snake_case_names() {
        assert_eq!(serde_json::to_string(&EventTopic::All).unwrap(), "\"all\"");
        assert_eq!(
            serde_json::to_string(&EventTopic::Connection).unwrap(),
            "\"connection\""
        );
        assert_eq!(
            serde_json::to_string(&EventTopic::Mount).unwrap(),
            "\"mount\""
        );
    }
}
