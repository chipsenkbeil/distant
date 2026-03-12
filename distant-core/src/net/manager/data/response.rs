use crate::auth::msg::Authentication;
use crate::protocol::TunnelDirection;
use serde::{Deserialize, Serialize};

use super::{
    ConnectionInfo, ConnectionList, ManagedTunnelId, ManagerAuthenticationId, ManagerChannelId,
    SemVer,
};
use crate::net::common::{ConnectionId, Destination, UntypedResponse};

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
}

impl<T: std::error::Error> From<T> for ManagerResponse {
    fn from(x: T) -> Self {
        Self::Error {
            description: x.to_string(),
        }
    }
}
