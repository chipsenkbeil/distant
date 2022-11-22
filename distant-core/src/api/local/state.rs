use crate::data::{ProcessId, SearchId};
use distant_net::common::ConnectionId;
use log::*;
use std::{io, path::PathBuf};

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

/// Holds connection-specific state managed by the server
#[derive(Default)]
pub struct ConnectionState {
    /// Unique id associated with connection
    id: ConnectionId,

    /// Channel connected to global process state
    pub(crate) process_channel: ProcessChannel,

    /// Channel connected to global search state
    pub(crate) search_channel: SearchChannel,

    /// Channel connected to global watcher state
    pub(crate) watcher_channel: WatcherChannel,

    /// Contains ids of processes that will be terminated when the connection is closed
    processes: Vec<ProcessId>,

    /// Contains paths being watched that will be unwatched when the connection is closed
    paths: Vec<PathBuf>,

    /// Contains ids of searches that will be terminated when the connection is closed
    searches: Vec<SearchId>,
}

impl Drop for ConnectionState {
    fn drop(&mut self) {
        let id = self.id;
        let processes: Vec<ProcessId> = self.processes.drain(..).collect();
        let paths: Vec<PathBuf> = self.paths.drain(..).collect();
        let searches: Vec<SearchId> = self.searches.drain(..).collect();

        let process_channel = self.process_channel.clone();
        let search_channel = self.search_channel.clone();
        let watcher_channel = self.watcher_channel.clone();

        // NOTE: We cannot (and should not) block during drop to perform cleanup,
        //       instead spawning a task that will do the cleanup async
        tokio::spawn(async move {
            let conn_id = id;
            info!("[Conn {conn_id}] Dropped, so cleaning up resources");

            for id in processes {
                trace!("[Conn {conn_id}] Killing proc {id}");

                // NOTE: Only kill a process if it is not marked as persistent, so force = false
                if let Err(x) = process_channel.kill(id, /* force */ false).await {
                    error!("[Conn {conn_id}] Failed to kill proc {id}: {x}");
                }
            }

            for id in searches {
                trace!("[Conn {conn_id}] Canceling search {id}");

                if let Err(x) = search_channel.cancel(id).await {
                    error!("[Conn {conn_id}] Failed to cancel search {id}: {x}");
                }
            }

            for path in paths {
                trace!("[Conn {conn_id}] Unwatching path {path:?}");

                if let Err(x) = watcher_channel.unwatch(id, path.as_path()).await {
                    error!("[Conn {conn_id}] Failed to unwatch {path:?}: {x}");
                }
            }
        });
    }
}
