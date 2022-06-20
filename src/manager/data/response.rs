use super::{Destination, ErrorKind, Stats};
use distant_core::{data::DistantResponseData, DistantMsg};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum ManagerResponse {
    /// Acknowledgement that a request was completed
    Ok,

    /// Indicates that some error occurred during a request
    Error {
        kind: ErrorKind,
        description: String,
    },

    /// Confirmation of a connection being established
    Connected { id: usize },

    /// Information about a specific connection
    Info {
        id: usize,
        destination: Destination,
        extra: HashMap<String, String>,
        stats: Stats,
    },

    /// List of connections in the form of id -> destination
    List {
        connections: HashMap<usize, Destination>,
    },

    /// Forward a response back to a specific connection that made a request
    Response {
        payload: DistantMsg<DistantResponseData>,
    },
}
