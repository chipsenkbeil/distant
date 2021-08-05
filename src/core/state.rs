use log::*;
use std::{collections::HashMap, fmt::Debug, hash::Hash};
use tokio::sync::{mpsc, oneshot};

/// Holds state related to multiple clients managed by a server
pub struct ServerState<ClientId>
where
    ClientId: Debug + Hash + PartialEq + Eq,
{
    /// Map of all processes running on the server
    pub processes: HashMap<usize, Process>,

    /// List of processes that will be killed when a client drops
    client_processes: HashMap<ClientId, Vec<usize>>,
}

impl<ClientId> ServerState<ClientId>
where
    ClientId: Debug + Hash + PartialEq + Eq,
{
    /// Pushes a new process associated with a client
    pub fn push_process(&mut self, client_id: ClientId, process: Process) {
        self.client_processes
            .entry(client_id)
            .or_insert(Vec::new())
            .push(process.id);
        self.processes.insert(process.id, process);
    }

    /// Cleans up state associated with a particular client
    pub async fn cleanup_client(&mut self, client_id: ClientId) {
        debug!("<Client @ {:?}> Cleaning up state", client_id);
        if let Some(ids) = self.client_processes.remove(&client_id) {
            for id in ids {
                if let Some(process) = self.processes.remove(&id) {
                    trace!(
                        "<Client @ {:?}> Requesting proc {} be killed",
                        client_id,
                        process.id
                    );
                    if let Err(_) = process.kill_tx.send(()) {
                        error!(
                            "Client {} failed to send process {} kill signal",
                            id, process.id
                        );
                    }
                }
            }
        }
    }
}

impl<ClientId> Default for ServerState<ClientId>
where
    ClientId: Debug + Hash + PartialEq + Eq,
{
    fn default() -> Self {
        Self {
            processes: HashMap::new(),
            client_processes: HashMap::new(),
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
