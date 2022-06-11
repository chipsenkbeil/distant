use super::{InputChannel, ProcessKiller, ProcessPty};
use crate::data::{ChangeKindSet, DistantResponseData};
use log::*;
use notify::RecommendedWatcher;
use std::{
    collections::HashMap,
    future::Future,
    hash::{Hash, Hasher},
    io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    pin::Pin,
};

pub type ReplyFn = Box<dyn FnMut(Vec<DistantResponseData>) -> ReplyRet + Send + 'static>;
pub type ReplyRet = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;

/// Holds state related to multiple connections managed by a server
#[derive(Default)]
pub struct State {
    /// Map of all processes running on the server
    pub processes: HashMap<usize, ProcessState>,

    /// List of processes that will be killed when a connection drops
    client_processes: HashMap<usize, Vec<usize>>,

    /// Watcher used for filesystem events
    pub watcher: Option<RecommendedWatcher>,

    /// Mapping of Path -> (Reply Fn, recursive) for watcher notifications
    pub watcher_paths: HashMap<WatcherPath, ReplyFn>,
}

#[derive(Clone, Debug)]
pub struct WatcherPath {
    /// The raw path provided to the watcher, which is not canonicalized
    raw_path: PathBuf,

    /// The canonicalized path at the time of providing to the watcher,
    /// as all paths must exist for a watcher, we use this to get the
    /// source of truth when watching
    path: PathBuf,

    /// Whether or not the path was set to be recursive
    recursive: bool,

    /// Specific filter for path
    only: ChangeKindSet,
}

impl PartialEq for WatcherPath {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for WatcherPath {}

impl Hash for WatcherPath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl Deref for WatcherPath {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl DerefMut for WatcherPath {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.path
    }
}

impl WatcherPath {
    /// Create a new watcher path using the given path and canonicalizing it
    pub fn new(
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
    ) -> io::Result<Self> {
        let raw_path = path.into();
        let path = raw_path.canonicalize()?;
        let only = only.into();
        Ok(Self {
            raw_path,
            path,
            recursive,
            only,
        })
    }

    pub fn raw_path(&self) -> &Path {
        self.raw_path.as_path()
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Returns true if this watcher path applies to the given path.
    /// This is accomplished by checking if the path is contained
    /// within either the raw or canonicalized path of the watcher
    /// and ensures that recursion rules are respected
    pub fn applies_to_path(&self, path: &Path) -> bool {
        let check_path = |path: &Path| -> bool {
            let cnt = path.components().count();

            // 0 means exact match from strip_prefix
            // 1 means that it was within immediate directory (fine for non-recursive)
            // 2+ means it needs to be recursive
            cnt < 2 || self.recursive
        };

        match (
            path.strip_prefix(self.path()),
            path.strip_prefix(self.raw_path()),
        ) {
            (Ok(p1), Ok(p2)) => check_path(p1) || check_path(p2),
            (Ok(p), Err(_)) => check_path(p),
            (Err(_), Ok(p)) => check_path(p),
            (Err(_), Err(_)) => false,
        }
    }
}

/// Holds information related to a spawned process on the server
pub struct ProcessState {
    pub cmd: String,
    pub args: Vec<String>,
    pub persist: bool,

    pub id: usize,
    pub stdin: Option<Box<dyn InputChannel>>,
    pub killer: Box<dyn ProcessKiller>,
    pub pty: Box<dyn ProcessPty>,
}

impl State {
    pub fn map_paths_to_watcher_paths_and_replies<'a>(
        &mut self,
        paths: &'a [PathBuf],
    ) -> Vec<(Vec<&'a PathBuf>, &ChangeKindSet, &mut ReplyFn)> {
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
    pub fn close_stdin_for_connection(&mut self, conn_id: usize) {
        debug!("<Conn @ {:?}> Closing stdin to all processes", conn_id);
        if let Some(ids) = self.client_processes.get(&conn_id) {
            for id in ids {
                if let Some(process) = self.processes.get_mut(id) {
                    trace!(
                        "<Conn @ {:?}> Closing stdin for proc {}",
                        conn_id,
                        process.id
                    );

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
                            "<Conn @ {:?}> Requesting proc {} be killed",
                            conn_id,
                            process.id
                        );
                        let pid = process.id;
                        if let Err(x) = process.killer.kill().await {
                            error!(
                                "Conn {} failed to send process {} kill signal: {}",
                                id, pid, x
                            );
                        }
                    } else {
                        trace!(
                            "<Conn @ {:?}> Proc {} is persistent and will not be killed",
                            conn_id,
                            process.id
                        );
                    }
                }
            }
        }
    }
}
