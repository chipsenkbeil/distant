use distant_auth::msg::Authentication;
use serde::{Deserialize, Serialize};

use super::{
    ConnectionInfo, ConnectionList, ManagerAuthenticationId, ManagerCapabilities, ManagerChannelId,
};
use crate::common::{ConnectionId, Destination, UntypedResponse};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerResponse {
    /// Acknowledgement that a connection was killed
    Killed,

    /// Indicates that some error occurred during a request
    Error { description: String },

    /// Response to retrieving information about the manager's capabilities
    Capabilities { supported: ManagerCapabilities },

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
}

impl<T: std::error::Error> From<T> for ManagerResponse {
    fn from(x: T) -> Self {
        Self::Error {
            description: x.to_string(),
        }
    }
}
