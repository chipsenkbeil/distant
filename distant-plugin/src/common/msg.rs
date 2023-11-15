use serde::{Deserialize, Serialize};

use crate::protocol;

/// Represents an id associated with a request or response.
pub type Id = u64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: Id,
    pub flags: RequestFlags,
    pub payload: protocol::Msg<protocol::Request>,
}

impl From<protocol::Msg<protocol::Request>> for Request {
    fn from(msg: protocol::Msg<protocol::Request>) -> Self {
        Self {
            id: rand::random(),
            flags: Default::default(),
            payload: msg,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestFlags {
    /// If true, payload should be executed in sequence; otherwise,
    /// a batch payload can be executed in any order.
    pub sequence: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub id: Id,
    pub origin: Id,
    pub payload: protocol::Msg<protocol::Response>,
}
