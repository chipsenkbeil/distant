use log::*;
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

/// Holds state related to multiple clients managed by a server
#[derive(Default)]
pub struct State {
    /// Map of all processes running on the server
    pub processes: HashMap<usize, Process>,

    /// List of processes that will be killed when a client drops
    client_processes: HashMap<usize, Vec<usize>>,
}

impl State {
    /// Pushes a new process associated with a client
    pub fn push_process(&mut self, conn_id: usize, process: Process) {
        self.client_processes
            .entry(conn_id)
            .or_insert(Vec::new())
            .push(process.id);
        self.processes.insert(process.id, process);
    }

    /// Cleans up state associated with a particular connection
    pub async fn cleanup_connection(&mut self, conn_id: usize) {
        debug!("<Conn @ {:?}> Cleaning up state", conn_id);
        if let Some(ids) = self.client_processes.remove(&conn_id) {
            for id in ids {
                if let Some(process) = self.processes.remove(&id) {
                    trace!(
                        "<Conn @ {:?}> Requesting proc {} be killed",
                        conn_id,
                        process.id
                    );
                    if let Err(_) = process.kill_tx.send(()) {
                        error!(
                            "Conn {} failed to send process {} kill signal",
                            id, process.id
                        );
                    }
                }
            }
        }
    }
}

/// Represents an actively-running process
pub struct Process {
    /// Id of the process
    pub id: usize,

    /// Command used to start the process
    pub cmd: String,

    /// Arguments associated with the process
    pub args: Vec<String>,

    /// Transport channel to send new input to the stdin of the process,
    /// one line at a time
    pub stdin_tx: mpsc::Sender<String>,

    /// Transport channel to report that the process should be killed
    pub kill_tx: oneshot::Sender<()>,
}
