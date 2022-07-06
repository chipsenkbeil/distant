use crate::{data::Error, ConnectionInfo, ConnectionList, Destination};
use crate::{ChannelId, ConnectionId, DistantMsg, DistantResponseData};
use distant_net::Response;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerResponse {
    /// Acknowledgement that a connection was killed
    Killed,

    /// Broadcast that the manager is shutting down (not guaranteed to be sent)
    Shutdown,

    /// Indicates that some error occurred during a request
    Error(Error),

    /// Confirmation of a distant server being launched
    Launched {
        /// Updated location of the spawned server
        destination: Destination,
    },

    /// Confirmation of a connection being established
    Connected { id: ConnectionId },

    /// Information about a specific connection
    Info(ConnectionInfo),

    /// List of connections in the form of id -> destination
    List(ConnectionList),

    /// Forward a response back to a specific channel that made a request
    Channel {
        /// Id of the channel
        id: ChannelId,

        /// Response to an earlier channel request
        response: Response<DistantMsg<DistantResponseData>>,
    },

    /// Indicates that a channel has been opened
    ChannelOpened {
        /// Id of the channel
        id: ChannelId,
    },

    /// Indicates that a channel has been closed
    ChannelClosed {
        /// Id of the channel
        id: ChannelId,
    },
}
