use std::io;

mod process;
pub use process::*;

mod search;
pub use search::*;

mod watcher;
pub use watcher::*;

/// Holds global state state managed by the server
pub struct GlobalState {
    /// State that holds information about processes running on the server
    pub process: ProcessState,

    /// State that holds information about searches running on the server
    pub search: SearchState,

    /// Watcher used for filesystem events
    pub watcher: WatcherState,
}

impl GlobalState {
    pub fn initialize() -> io::Result<Self> {
        Ok(Self {
            process: ProcessState::new(),
            search: SearchState::new(),
            watcher: WatcherState::initialize()?,
        })
    }
}
