use super::{ConnectionId, Destination};
use crate::data::Map;
use serde::{Deserialize, Serialize};

/// Information about a specific connection
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// Connection's id
    pub id: ConnectionId,

    /// Destination with which this connection is associated
    pub destination: Destination,

    /// Additional options associated with this connection
    pub options: Map,
}
