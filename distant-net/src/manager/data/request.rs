use crate::common::{ConnectionId, Destination, Map};
use derive_more::IsVariant;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumDiscriminants, EnumIter, EnumMessage, EnumString};

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, EnumDiscriminants, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::Subcommand))]
#[strum_discriminants(derive(
    AsRefStr,
    strum::Display,
    EnumIter,
    EnumMessage,
    EnumString,
    Hash,
    PartialOrd,
    Ord,
    IsVariant,
    Serialize,
    Deserialize
))]
#[cfg_attr(
    feature = "schemars",
    strum_discriminants(derive(schemars::JsonSchema))
)]
#[strum_discriminants(name(ManagerCapabilityKind))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerRequest {
    /// Retrieve information about the server's capabilities
    #[strum_discriminants(strum(message = "Supports retrieving capabilities"))]
    Capabilities,

    /// Launch a server using the manager
    #[strum_discriminants(strum(message = "Supports launching a server on remote machines"))]
    Launch {
        // NOTE: Boxed per clippy's large_enum_variant warning
        destination: Box<Destination>,

        /// Additional options specific to the connection
        #[cfg_attr(feature = "clap", clap(short, long, action = clap::ArgAction::Append))]
        options: Map,
    },

    /// Initiate a connection through the manager
    #[strum_discriminants(strum(message = "Supports connecting to remote servers"))]
    Connect {
        // NOTE: Boxed per clippy's large_enum_variant warning
        destination: Box<Destination>,

        /// Additional options specific to the connection
        #[cfg_attr(feature = "clap", clap(short, long, action = clap::ArgAction::Append))]
        options: Map,
    },

    /// Opens a channel for communication with a server
    #[cfg_attr(feature = "clap", clap(skip))]
    #[strum_discriminants(strum(message = "Supports opening a channel with a remote server"))]
    OpenChannel {
        /// Id of the connection
        id: ConnectionId,
    },

    /// Sends data through channel
    #[cfg_attr(feature = "clap", clap(skip))]
    #[strum_discriminants(strum(
        message = "Supports sending data through a channel with a remote server"
    ))]
    Channel {
        /// Id of the channel
        id: ChannelId,

        /// Raw data to send through the channel
        data: Vec<u8>,
    },

    /// Closes an open channel
    #[cfg_attr(feature = "clap", clap(skip))]
    #[strum_discriminants(strum(message = "Supports closing a channel with a remote server"))]
    CloseChannel {
        /// Id of the channel to close
        id: ChannelId,
    },

    /// Retrieve information about a specific connection
    #[strum_discriminants(strum(message = "Supports retrieving connection-specific information"))]
    Info { id: ConnectionId },

    /// Kill a specific connection
    #[strum_discriminants(strum(message = "Supports killing a remote connection"))]
    Kill { id: ConnectionId },

    /// Retrieve list of connections being managed
    #[strum_discriminants(strum(message = "Supports retrieving a list of managed connections"))]
    List,
}
