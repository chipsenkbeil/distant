use std::io;

mod process;
pub use process::*;

mod watcher;
pub use watcher::*;

/// Holds global state state managed by the server
pub struct GlobalState {
    /// State that holds information about processes running on the server
    pub process: ProcessState,

    /// Watcher used for filesystem events
    pub watcher: WatcherState,
}

impl GlobalState {
    pub fn initialize() -> io::Result<Self> {
        Ok(Self {
            process: ProcessState::new(),
            watcher: WatcherState::initialize()?,
        })
    }
}
