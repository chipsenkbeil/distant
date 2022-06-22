use super::{Destination, Extra};
use serde::{Deserialize, Serialize};

/// Information about a specific connection
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// Connection's id
    pub id: usize,

    /// Destination with which this connection is associated
    pub destination: Destination,

    /// Extra information associated with this connection
    pub extra: Extra,
}
