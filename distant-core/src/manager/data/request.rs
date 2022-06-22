use super::{Destination, Extra};
use clap::{ArgAction, Subcommand};
use distant_core::{data::DistantRequestData, DistantMsg};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Subcommand)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerRequest {
    /// Initiate a connection through the manager
    Connect {
        destination: Destination,

        /// Extra details specific to the connection
        #[clap(short, long, action = ArgAction::Append)]
        extra: Extra,
    },

    /// Forward a request to a specific connection
    Request {
        id: usize,

        #[clap(subcommand)]
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
