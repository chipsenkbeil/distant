use std::{collections::HashMap, io};
use tokio::sync::RwLock;

mod process;
pub use process::*;

mod watcher;
pub use watcher::*;

/// Holds global state state managed by the server
pub struct GlobalState {
    /// Map of all processes running on the server by their id
    pub processes: RwLock<HashMap<usize, ProcessState>>,

    /// Watcher used for filesystem events
    pub watcher: WatcherState,
}

impl GlobalState {
    pub fn initialize() -> io::Result<Self> {
        Ok(Self {
            processes: RwLock::new(HashMap::new()),
            watcher: WatcherState::initialize()?,
        })
    }
}
