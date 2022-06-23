use crate::{data::Error, ConnectionInfo, ConnectionList};
use crate::{DistantMsg, DistantResponseData};
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

    /// Confirmation of a connection being established
    Connected { id: usize },

    /// Information about a specific connection
    Info(ConnectionInfo),

    /// List of connections in the form of id -> destination
    List(ConnectionList),

    /// Forward a response back to a specific channel that made a request
    Channel {
        /// Id of the channel
        id: usize,

        /// Response to an earlier channel request
        payload: DistantMsg<DistantResponseData>,
    },

    /// Indicates that a channel has been opened
    ChannelOpened {
        /// Id of the channel
        id: usize,
    },

    /// Indicates that a channel has been closed
    ChannelClosed {
        /// Id of the channel
        id: usize,
    },
}
