use super::{Destination, Extra};
use crate::{DistantMsg, DistantRequestData};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::Subcommand))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerRequest {
    /// Initiate a connection through the manager
    Connect {
        // NOTE: Boxed per clippy's large_enum_variant warning
        destination: Box<Destination>,

        /// Extra details specific to the connection
        #[cfg_attr(feature = "clap", clap(short, long, action = clap::ArgAction::Append))]
        extra: Extra,
    },

    /// Forward a request to a specific connection
    Request {
        id: usize,

        #[cfg_attr(feature = "clap", clap(subcommand))]
        payload: DistantMsg<DistantRequestData>,
    },

    /// Retrieve information about a specific connection
    Info { id: usize },

    /// Kill a specific connection
    Kill { id: usize },

    /// Retrieve list of connections being managed
    List,

    /// Signals the manager to shutdown
    Shutdown,
}
