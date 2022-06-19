use std::{io, path::PathBuf};

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

/// Holds connection-specific state managed by the server
#[derive(Default)]
pub struct ConnectionState {
    /// Unique id associated with connection
    id: usize,

    /// Channel connected to global process state
    pub(crate) process_channel: ProcessChannel,

    /// Channel connected to global watcher state
    pub(crate) watcher_channel: WatcherChannel,

    /// Contains ids of processes that will be terminated when the connection is closed
    processes: Vec<usize>,

    /// Contains paths being watched that will be unwatched when the connection is closed
    paths: Vec<PathBuf>,
}

impl Drop for ConnectionState {
    fn drop(&mut self) {
        let id = self.id;
        let processes: Vec<usize> = self.processes.drain(..).collect();
        let paths: Vec<PathBuf> = self.paths.drain(..).collect();

        let process_channel = self.process_channel.clone();
        let watcher_channel = self.watcher_channel.clone();

        // NOTE: We cannot (and should not) block during drop to perform cleanup,
        //       instead spawning a task that will do the cleanup async
        tokio::spawn(async move {
            for id in processes {
                let _ = process_channel.kill(id).await;
            }

            for path in paths {
                let _ = watcher_channel.unwatch(id, path).await;
            }
        });
    }
}
