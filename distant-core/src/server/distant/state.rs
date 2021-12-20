use crate::data::PtySize;
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
            .or_insert_with(Vec::new)
            .push(process.id);
        self.processes.insert(process.id, process);
    }

    pub fn mut_process(&mut self, proc_id: usize) -> Option<&mut Process> {
        self.processes.get_mut(&proc_id)
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
                    if !process.detached {
                        trace!(
                            "<Conn @ {:?}> Requesting proc {} be killed",
                            conn_id,
                            process.id
                        );
                        let pid = process.id;
                        if !process.kill() {
                            error!("Conn {} failed to send process {} kill signal", id, pid);
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

/// Represents an actively-running process
pub struct Process {
    /// Id of the process
    pub id: usize,

    /// Command used to start the process
    pub cmd: String,

    /// Arguments associated with the process
    pub args: Vec<String>,

    /// Whether or not this process was run detached
    pub detached: bool,

    /// Dimensions of pty associated with process, if it has one
    pub pty: Option<PtySize>,

    /// Transport channel to send new input to the stdin of the process,
    /// one line at a time
    stdin_tx: Option<mpsc::Sender<Vec<u8>>>,

    /// Transport channel to report that the process should be killed
    kill_tx: Option<oneshot::Sender<()>>,

    /// Task used to wait on the process to complete or be killed
    wait_task: Option<JoinHandle<()>>,
}

impl Process {
    pub fn new(
        id: usize,
        cmd: String,
        args: Vec<String>,
        detached: bool,
        pty: Option<PtySize>,
    ) -> Self {
        Self {
            id,
            cmd,
            args,
            detached,
            pty,
            stdin_tx: None,
            kill_tx: None,
            wait_task: None,
        }
    }

    /// Lazy initialization of process state
    pub(crate) fn initialize(
        &mut self,
        stdin_tx: mpsc::Sender<Vec<u8>>,
        kill_tx: oneshot::Sender<()>,
        wait_task: JoinHandle<()>,
    ) {
        self.stdin_tx = Some(stdin_tx);
        self.kill_tx = Some(kill_tx);
        self.wait_task = Some(wait_task);
    }

    pub async fn send_stdin(&self, input: impl Into<Vec<u8>>) -> bool {
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

    pub fn kill(self) -> bool {
        self.kill_tx
            .map(|tx| tx.send(()).is_ok())
            .unwrap_or_default()
    }
}

impl Future for Process {
    type Output = Result<(), JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(task) = self.wait_task.as_mut() {
            Pin::new(task).poll(cx)
        } else {
            // TODO: Does this work?
            Poll::Pending
        }
    }
}
