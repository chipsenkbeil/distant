use crate::{
    api::local::state::{ProcessState, WatcherPath},
    data::DistantResponseData,
};
use distant_net::QueuedServerReply;
use std::collections::HashMap;

/// Holds state related to a connection managed by the server
#[derive(Default)]
pub struct ConnectionState {
    /// List of processes that will be killed when a connection drops
    pub processes: HashMap<usize, ProcessState>,

    /// Mapping of Path -> Sender for watcher notifications
    pub watcher_paths: HashMap<WatcherPath, QueuedServerReply<DistantResponseData>>,
}
