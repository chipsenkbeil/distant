use super::{ChannelId, ConnectionId, Destination};
use crate::{DistantMsg, DistantRequestData, Map};
use distant_net::Request;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::Subcommand))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerRequest {
    /// Launch a server using the manager
    Launch {
        // NOTE: Boxed per clippy's large_enum_variant warning
        destination: Box<Destination>,

        /// Additional options specific to the connection
        #[cfg_attr(feature = "clap", clap(short, long, action = clap::ArgAction::Append))]
        options: Map,
    },

    /// Initiate a connection through the manager
    Connect {
        // NOTE: Boxed per clippy's large_enum_variant warning
        destination: Box<Destination>,

        /// Additional options specific to the connection
        #[cfg_attr(feature = "clap", clap(short, long, action = clap::ArgAction::Append))]
        options: Map,
    },

    /// Opens a channel for communication with a server
    #[cfg_attr(feature = "clap", clap(skip))]
    OpenChannel {
        /// Id of the connection
        id: ConnectionId,
    },

    /// Sends data through channel
    #[cfg_attr(feature = "clap", clap(skip))]
    Channel {
        /// Id of the channel
        id: ChannelId,

        /// Request to send to through the channel
        #[cfg_attr(feature = "clap", clap(skip = skipped_request()))]
        request: Request<DistantMsg<DistantRequestData>>,
    },

    /// Closes an open channel
    #[cfg_attr(feature = "clap", clap(skip))]
    CloseChannel {
        /// Id of the channel to close
        id: ChannelId,
    },

    /// Retrieve information about a specific connection
    Info { id: ConnectionId },

    /// Kill a specific connection
    Kill { id: ConnectionId },

    /// Retrieve list of connections being managed
    List,

    /// Signals the manager to shutdown
    Shutdown,
}

/// Produces some default request, purely to satisfy clap
#[cfg(feature = "clap")]
fn skipped_request() -> Request<DistantMsg<DistantRequestData>> {
    Request::new(DistantMsg::Single(DistantRequestData::SystemInfo {}))
}
