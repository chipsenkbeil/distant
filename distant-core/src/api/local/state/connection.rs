use crate::api::local::state::GlobalState;
use distant_net::QueuedServerReply;
use notify::{RecursiveMode, Watcher};
use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    io,
    path::Path,
    sync::Weak,
};

mod path;
pub use path::*;

mod process;
pub use process::*;

/// Holds state related to a connection managed by the server
#[derive(Default)]
pub struct ConnectionState {
    /// Reference to the global state
    pub global_state: Weak<GlobalState>,

    /// List of processes that will be killed when a connection drops
    pub processes: HashMap<usize, ProcessState>,

    /// Collection of paths being watched by this connection
    pub paths: HashSet<RegisteredPath>,
}

impl ConnectionState {
    pub async fn watch(&mut self, path: RegisteredPath) -> io::Result<()> {
        let global_state = Weak::upgrade(&self.global_state)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Global state is unavailable"))?;

        global_state.watcher.lock().await.watch(
            path.path(),
            if path.is_recursive() {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            },
        )?;

        self.paths.insert(path);

        Ok(())
    }

    pub async fn unwatch(&mut self, path: impl AsRef<Path>) -> io::Result<()> {
        let global_state = Weak::upgrade(&self.global_state)
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Global state is unavailable"))?;

        global_state.watcher.unwatch(path.as_ref()).map_err()?;

        // Remove the path from our list associated with the connection
        if let Some(paths) = self.paths.lock().await.get_mut(&connection_id) {
            let path = path.as_ref();
            paths.retain(|wp| wp.path == path || wp.raw_path == path);
        }

        Ok(())
    }
}
