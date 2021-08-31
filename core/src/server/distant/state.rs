use log::*;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::{JoinError, JoinHandle},
};

/// Holds state related to multiple connections managed by a server
#[derive(Default)]
pub struct State {
    /// Map of all processes running on the server
    pub processes: HashMap<usize, Process>,

    /// List of processes that will be killed when a connection drops
    client_processes: HashMap<usize, Vec<usize>>,
}

impl State {
    /// Pushes a new process associated with a connection
    pub fn push_process(&mut self, conn_id: usize, process: Process) {
        self.client_processes
            .entry(conn_id)
            .or_insert(Vec::new())
            .push(process.id);
        self.processes.insert(process.id, process);
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
                if let Some(process) = self.processes.get_mut(&id) {
                    trace!(
                        "<Conn @ {:?}> Closing stdin for proc {}",
                        conn_id,
                        process.id
                    );

                    process.close_stdin();
                }
            }
        }
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
    pub stdin_tx: Option<mpsc::Sender<String>>,

    /// Transport channel to report that the process should be killed
    pub kill_tx: oneshot::Sender<()>,

    /// Task used to wait on the process to complete or be killed
    pub wait_task: JoinHandle<()>,
}

impl Process {
    pub async fn send_stdin(&self, input: impl Into<String>) -> bool {
        if let Some(stdin) = self.stdin_tx.as_ref() {
            if stdin.send(input.into()).await.is_ok() {
                return true;
            }
        }

        false
    }

    pub fn close_stdin(&mut self) {
        self.stdin_tx.take();
    }

    pub async fn kill_and_wait(self) -> Result<(), JoinError> {
        let _ = self.kill_tx.send(());
        self.wait_task.await
    }
}

impl Future for Process {
    type Output = Result<(), JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.wait_task).poll(cx)
    }
}
