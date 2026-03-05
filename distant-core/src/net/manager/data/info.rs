use serde::{Deserialize, Serialize};

use crate::net::common::{ConnectionId, Map};

/// Information about a specific connection
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// Connection's id
    pub id: ConnectionId,

    /// Raw destination string (e.g. `"docker://ubuntu:22.04"` or `"ssh://host:22"`).
    pub destination: String,

    /// Additional options associated with this connection
    pub options: Map,
}
