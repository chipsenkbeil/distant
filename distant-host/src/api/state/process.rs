use std::collections::HashMap;
use std::io;
use std::ops::Deref;
use std::path::PathBuf;

use distant_core::net::server::Reply;
use distant_core::protocol::{Environment, ProcessId, PtySize, Response};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

mod instance;
pub use instance::*;

/// Holds information related to spawned processes on the server.
pub struct ProcessState {
    channel: ProcessChannel,
    task: JoinHandle<()>,
}

impl Drop for ProcessState {
    /// Aborts the task that handles process operations and management.
    fn drop(&mut self) {
        self.abort();
    }
}

impl ProcessState {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(process_task(tx.clone(), rx));

        Self {
            channel: ProcessChannel { tx },
            task,
        }
    }

    /// Aborts the process task
    pub fn abort(&self) {
        self.task.abort();
    }
}

impl Deref for ProcessState {
    type Target = ProcessChannel;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

#[derive(Clone)]
pub struct ProcessChannel {
    tx: mpsc::Sender<InnerProcessMsg>,
}

impl Default for ProcessChannel {
    /// Creates a new channel that is closed by default.
    fn default() -> Self {
        let (tx, _) = mpsc::channel(1);
        Self { tx }
    }
}

impl ProcessChannel {
    /// Spawns a new process, returning the id associated with it.
    pub async fn spawn(
        &self,
        cmd: String,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
        reply: Box<dyn Reply<Data = Response>>,
    ) -> io::Result<ProcessId> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerProcessMsg::Spawn {
                cmd,
                environment,
                current_dir,
                pty,
                reply,
                cb,
            })
            .await
            .map_err(|_| io::Error::other("Internal process task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to spawn dropped"))?
    }

    /// Resizes the pty of a running process.
    pub async fn resize_pty(&self, id: ProcessId, size: PtySize) -> io::Result<()> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerProcessMsg::Resize { id, size, cb })
            .await
            .map_err(|_| io::Error::other("Internal process task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to resize dropped"))?
    }

    /// Send stdin to a running process.
    pub async fn send_stdin(&self, id: ProcessId, data: Vec<u8>) -> io::Result<()> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerProcessMsg::Stdin { id, data, cb })
            .await
            .map_err(|_| io::Error::other("Internal process task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to stdin dropped"))?
    }

    /// Kills a running process, including persistent processes if `force` is true. Will fail if
    /// unable to kill the process or `force` is false when the process is persistent.
    pub async fn kill(&self, id: ProcessId) -> io::Result<()> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerProcessMsg::Kill { id, cb })
            .await
            .map_err(|_| io::Error::other("Internal process task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to kill dropped"))?
    }
}

/// Internal message to pass to our task below to perform some action.
enum InnerProcessMsg {
    Spawn {
        cmd: String,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
        reply: Box<dyn Reply<Data = Response>>,
        cb: oneshot::Sender<io::Result<ProcessId>>,
    },
    Resize {
        id: ProcessId,
        size: PtySize,
        cb: oneshot::Sender<io::Result<()>>,
    },
    Stdin {
        id: ProcessId,
        data: Vec<u8>,
        cb: oneshot::Sender<io::Result<()>>,
    },
    Kill {
        id: ProcessId,
        cb: oneshot::Sender<io::Result<()>>,
    },
    InternalRemove {
        id: ProcessId,
    },
}

async fn process_task(tx: mpsc::Sender<InnerProcessMsg>, mut rx: mpsc::Receiver<InnerProcessMsg>) {
    let mut processes: HashMap<ProcessId, ProcessInstance> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        match msg {
            InnerProcessMsg::Spawn {
                cmd,
                environment,
                current_dir,
                pty,
                reply,
                cb,
            } => {
                let _ = cb.send(
                    match ProcessInstance::spawn(cmd, environment, current_dir, pty, reply) {
                        Ok(mut process) => {
                            let id = process.id;

                            // Attach a callback for when the process is finished where
                            // we will remove it from our above list
                            let tx = tx.clone();
                            process.on_done(move |_| async move {
                                let _ = tx.send(InnerProcessMsg::InternalRemove { id }).await;
                            });

                            processes.insert(id, process);
                            Ok(id)
                        }
                        Err(x) => Err(x),
                    },
                );
            }
            InnerProcessMsg::Resize { id, size, cb } => {
                let _ = cb.send(match processes.get(&id) {
                    Some(process) => process.pty.resize_pty(size),
                    None => Err(io::Error::other(format!("No process found with id {id}"))),
                });
            }
            InnerProcessMsg::Stdin { id, data, cb } => {
                let _ = cb.send(match processes.get_mut(&id) {
                    Some(process) => match process.stdin.as_mut() {
                        Some(stdin) => stdin.send(&data).await,
                        None => Err(io::Error::other(format!("Process {id} stdin is closed"))),
                    },
                    None => Err(io::Error::other(format!("No process found with id {id}"))),
                });
            }
            InnerProcessMsg::Kill { id, cb } => {
                let _ = cb.send(match processes.get_mut(&id) {
                    Some(process) => process.killer.kill().await,
                    None => Err(io::Error::other(format!("No process found with id {id}"))),
                });
            }
            InnerProcessMsg::InternalRemove { id } => {
                processes.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    // ---- ProcessChannel::default ----

    #[test(tokio::test)]
    async fn default_channel_spawn_should_fail_with_closed_error() {
        let channel = ProcessChannel::default();
        let (reply, _rx) = tokio::sync::mpsc::unbounded_channel();

        let result = channel
            .spawn(
                "echo hello".to_string(),
                Environment::new(),
                None,
                None,
                Box::new(reply),
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Internal process task closed"),
            "Unexpected error: {}",
            err
        );
    }

    #[test(tokio::test)]
    async fn default_channel_resize_pty_should_fail_with_closed_error() {
        let channel = ProcessChannel::default();
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        let result = channel.resize_pty(1, size).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Internal process task closed"));
    }

    #[test(tokio::test)]
    async fn default_channel_send_stdin_should_fail_with_closed_error() {
        let channel = ProcessChannel::default();
        let result = channel.send_stdin(1, b"data".to_vec()).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Internal process task closed"));
    }

    #[test(tokio::test)]
    async fn default_channel_kill_should_fail_with_closed_error() {
        let channel = ProcessChannel::default();
        let result = channel.kill(1).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Internal process task closed"));
    }

    // ---- ProcessState ----

    #[test(tokio::test)]
    async fn process_state_new_should_create_working_channel() {
        let state = ProcessState::new();

        // Should be able to spawn a real process (e.g., echo)
        let (reply, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let cmd = if cfg!(windows) {
            "cmd /C echo hello".to_string()
        } else {
            "echo hello".to_string()
        };

        let id = state
            .spawn(cmd, Environment::new(), None, None, Box::new(reply))
            .await
            .unwrap();

        assert!(id > 0);

        // Collect responses - we should get at least ProcStdout and ProcDone
        let mut got_done = false;
        while let Some(resp) = rx.recv().await {
            match resp {
                Response::ProcDone { id: done_id, .. } => {
                    assert_eq!(done_id, id);
                    got_done = true;
                    break;
                }
                Response::ProcStdout { id: stdout_id, .. } => {
                    assert_eq!(stdout_id, id);
                }
                _ => {}
            }
        }
        assert!(got_done, "Never received ProcDone response");
    }

    #[test(tokio::test)]
    async fn process_state_deref_provides_channel() {
        let state = ProcessState::new();

        // Verify Deref works by accessing channel methods
        let (reply, _rx) = tokio::sync::mpsc::unbounded_channel();

        let cmd = if cfg!(windows) {
            "cmd /C echo test".to_string()
        } else {
            "echo test".to_string()
        };

        // This uses Deref to call spawn on the ProcessChannel
        let result = state
            .spawn(cmd, Environment::new(), None, None, Box::new(reply))
            .await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn process_state_abort_should_close_the_internal_task() {
        let state = ProcessState::new();
        state.abort();

        // After aborting, operations should fail
        // Give a brief moment for the abort to propagate
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let (reply, _rx) = tokio::sync::mpsc::unbounded_channel();
        let result = state
            .spawn(
                "echo test".to_string(),
                Environment::new(),
                None,
                None,
                Box::new(reply),
            )
            .await;
        assert!(result.is_err());
    }

    #[test(tokio::test)]
    async fn resize_pty_should_fail_for_nonexistent_process() {
        let state = ProcessState::new();
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        let result = state.resize_pty(99999, size).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No process found with id 99999"));
    }

    #[test(tokio::test)]
    async fn send_stdin_should_fail_for_nonexistent_process() {
        let state = ProcessState::new();
        let result = state.send_stdin(99999, b"data".to_vec()).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No process found with id 99999"));
    }

    #[test(tokio::test)]
    async fn kill_should_fail_for_nonexistent_process() {
        let state = ProcessState::new();
        let result = state.kill(99999).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No process found with id 99999"));
    }

    #[test(tokio::test)]
    async fn kill_should_succeed_for_running_process() {
        let state = ProcessState::new();
        let (reply, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn a long-running process
        let cmd = if cfg!(windows) {
            "cmd /C ping -n 100 127.0.0.1".to_string()
        } else {
            "sleep 60".to_string()
        };

        let id = state
            .spawn(cmd, Environment::new(), None, None, Box::new(reply))
            .await
            .unwrap();

        // Give it a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Kill should succeed
        let result = state.kill(id).await;
        assert!(result.is_ok());

        // Should eventually get a ProcDone
        let mut got_done = false;
        while let Some(resp) = rx.recv().await {
            if matches!(resp, Response::ProcDone { .. }) {
                got_done = true;
                break;
            }
        }
        assert!(got_done, "Never received ProcDone after kill");
    }

    #[test(tokio::test)]
    async fn send_stdin_should_succeed_for_running_process() {
        let state = ProcessState::new();
        let (reply, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn cat which reads from stdin
        let cmd = if cfg!(windows) {
            "cmd /C findstr x]^[".to_string()
        } else {
            "cat".to_string()
        };

        let id = state
            .spawn(cmd, Environment::new(), None, None, Box::new(reply))
            .await
            .unwrap();

        // Give it a moment to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Send some stdin
        let result = state.send_stdin(id, b"hello\n".to_vec()).await;
        assert!(result.is_ok());

        // Kill process to clean up
        let _ = state.kill(id).await;

        // Drain responses
        while rx.recv().await.is_some() {}
    }

    #[test(tokio::test)]
    async fn spawn_should_fail_for_empty_command() {
        let state = ProcessState::new();
        let (reply, _rx) = tokio::sync::mpsc::unbounded_channel();

        let result = state
            .spawn(
                "".to_string(),
                Environment::new(),
                None,
                None,
                Box::new(reply),
            )
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Command was empty"));
    }

    #[test(tokio::test)]
    async fn process_channel_clone_should_work() {
        let state = ProcessState::new();
        let channel: ProcessChannel = state.channel.clone();

        // The cloned channel should still be able to communicate
        let (reply, _rx) = tokio::sync::mpsc::unbounded_channel();

        let cmd = if cfg!(windows) {
            "cmd /C echo clone_test".to_string()
        } else {
            "echo clone_test".to_string()
        };

        let result = channel
            .spawn(cmd, Environment::new(), None, None, Box::new(reply))
            .await;
        assert!(result.is_ok());
    }
}
