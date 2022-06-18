use crate::api::local::process::{InputChannel, ProcessKiller, ProcessPty};
use log::*;

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

impl Drop for ProcessState {
    /// Closes stdin and attempts to kill the process when dropped
    fn drop(&mut self) {
        // Drop stdin first to close it
        self.stdin = None;

        // Attempt to kill the process, which is an async operation that we
        // will spawn a task to handle
        let id = self.id;
        let killer = self.killer.clone_killer();
        tokio::spawn(async move {
            if let Err(x) = killer.kill().await {
                error!("Failed to kill process {} when dropped: {}", id, x);
            }
        });
    }
}
