use super::{InputChannel, ProcessKiller, ProcessPty};
use log::*;
use std::collections::HashMap;

/// Holds state related to multiple connections managed by a server
#[derive(Default)]
pub struct State {
    /// Map of all processes running on the server
    pub processes: HashMap<usize, ProcessState>,

    /// List of processes that will be killed when a connection drops
    client_processes: HashMap<usize, Vec<usize>>,
}

/// Holds information related to a spawned process on the server
pub struct ProcessState {
    pub cmd: String,
    pub args: Vec<String>,
    pub detached: bool,

    pub id: usize,
    pub stdin: Option<Box<dyn InputChannel>>,
    pub killer: Box<dyn ProcessKiller>,
    pub pty: Box<dyn ProcessPty>,
}

impl State {
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
                    if !process.detached {
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
                            "<Conn @ {:?}> Proc {} is detached and will not be killed",
                            conn_id,
                            process.id
                        );
                    }
                }
            }
        }
    }
}
