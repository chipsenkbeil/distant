use std::collections::HashMap;
use tokio::sync::{Mutex, RwLock};

mod connection;
pub use connection::*;

mod process;
pub use process::*;

mod watcher;
pub use watcher::*;

/// Holds global state state managed by the server
#[derive(Default)]
pub struct GlobalState {
    /// Map of all processes running on the server
    processes: RwLock<HashMap<usize, ProcessState>>,

    /// Watcher used for filesystem events
    watcher: Mutex<WatcherState>,
}

pub struct GlobalWatcherState {}
