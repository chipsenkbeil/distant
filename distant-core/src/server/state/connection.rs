use crate::{data::DistantResponseData, server::state::WatcherPath};
use distant_net::QueuedServerReply;
use std::collections::HashMap;

/// Holds state related to a connection managed by the server
#[derive(Default)]
pub struct ConnectionState {
    /// List of processes that will be killed when a connection drops
    pub(crate) client_processes: HashMap<usize, Vec<usize>>,

    /// Mapping of Path -> Sender for watcher notifications
    pub watcher_paths: HashMap<WatcherPath, QueuedServerReply<DistantResponseData>>,
}
