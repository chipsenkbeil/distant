use crate::{
    data::{ChangeKindSet, DistantResponseData},
    server::process::ProcessKiller,
};
use distant_net::QueuedServerReply;
use log::*;
use notify::RecommendedWatcher;
use std::{collections::HashMap, path::PathBuf};
use tokio::sync::RwLock;

mod connection;
pub use connection::*;

mod process;
pub use process::*;

mod watcher;
pub use watcher::*;

/// Holds global state state managed by the server
#[derive(Default)]
pub struct GlobalState {
    /// Map of all active connections
    pub connections: RwLock<HashMap<usize, ConnectionState>>,

    /// Map of all processes running on the server
    pub processes: RwLock<HashMap<usize, ProcessState>>,

    /// Watcher used for filesystem events
    pub watcher: RwLock<Option<RecommendedWatcher>>,
}

impl GlobalState {
    pub fn map_paths_to_watcher_paths_and_replies<'a>(
        &mut self,
        paths: &'a [PathBuf],
    ) -> Vec<(
        Vec<&'a PathBuf>,
        &ChangeKindSet,
        QueuedServerReply<DistantResponseData>,
    )> {
        let mut results = Vec::new();

        for (wp, reply) in self.watcher_paths.iter_mut() {
            let mut wp_paths = Vec::new();
            for path in paths {
                if wp.applies_to_path(path) {
                    wp_paths.push(path);
                }
            }
            if !wp_paths.is_empty() {
                results.push((wp_paths, &wp.only, reply));
            }
        }

        results
    }

    /// Pushes a new process associated with a connection
    pub fn push_process_state(&mut self, conn_id: usize, process_state: ProcessState) {
        self.client_processes
            .entry(conn_id)
            .or_insert_with(Vec::new)
            .push(process_state.id);
        self.processes.insert(process_state.id, process_state);
    }

    /// Removes a process associated with a connection
    pub fn remove_process(&mut self, conn_id: usize, proc_id: usize) {
        self.client_processes.entry(conn_id).and_modify(|v| {
            if let Some(pos) = v.iter().position(|x| *x == proc_id) {
                v.remove(pos);
            }
        });
        self.processes.remove(&proc_id);
    }

    /// Closes stdin for all processes associated with the connection
    pub async fn close_stdin_for_connection(&mut self, conn_id: usize) {
        debug!("<Conn @ {:?}> Closing stdin to all processes", conn_id);
        if let Some(ids) = self.client_processes.get(&conn_id) {
            for id in ids {
                if let Some(process) = self.processes.get_mut(id) {
                    trace!("[Conn {}] Closing stdin for proc {}", conn_id, process.id);

                    let _ = process.stdin.take();
                }
            }
        }
    }

    /// Cleans up state associated with a particular connection
    pub async fn cleanup_connection(&mut self, conn_id: usize) {
        debug!("<Conn @ {:?}> Cleaning up state", conn_id);
        if let Some(ids) = self.client_processes.remove(&conn_id) {
            for id in ids {
                if let Some(mut process) = self.processes.remove(&id) {
                    if !process.persist {
                        trace!(
                            "[Conn {}] Requesting proc {} be killed",
                            conn_id,
                            process.id
                        );
                        let pid = process.id;
                        if let Err(x) = process.killer.kill().await {
                            error!(
                                "[Conn {}] Failed to send process {} kill signal: {}",
                                id, pid, x
                            );
                        }
                    } else {
                        trace!(
                            "[Conn {}] Proc {} is persistent and will not be killed",
                            conn_id,
                            process.id
                        );
                    }
                }
            }
        }
    }
}
