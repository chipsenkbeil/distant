use crate::{cli::client::stdin, constants::MAX_PIPE_CHUNK_SIZE};
use distant_core::{
    RemoteLspStderr, RemoteLspStdin, RemoteLspStdout, RemoteStderr, RemoteStdin, RemoteStdout,
};
use std::{
    io::{self, Write},
    thread,
};
use tokio::task::{JoinError, JoinHandle};

/// Represents a link between a remote process' stdin/stdout/stderr and this process'
/// stdin/stdout/stderr
pub struct RemoteProcessLink {
    _stdin_thread: Option<thread::JoinHandle<()>>,
    stdin_task: Option<JoinHandle<io::Result<()>>>,
    stdout_task: JoinHandle<io::Result<()>>,
    stderr_task: JoinHandle<io::Result<()>>,
}

macro_rules! from_pipes {
    ($stdin:expr, $stdout:expr, $stderr:expr) => {{
        let mut stdin_thread = None;
        let mut stdin_task = None;
        if let Some(mut stdin_handle) = $stdin {
            let (thread, mut rx) = stdin::spawn_channel(MAX_PIPE_CHUNK_SIZE);
            let task = tokio::spawn(async move {
                loop {
                    if let Some(input) = rx.recv().await {
                        if let Err(x) = stdin_handle.write(&*input).await {
                            break Err(x);
                        }
                    } else {
                        break Ok(());
                    }
                }
            });
            stdin_thread = Some(thread);
            stdin_task = Some(task);
        }
        let stdout_task = tokio::spawn(async move {
            let handle = io::stdout();
            loop {
                match $stdout.read().await {
                    Ok(output) => {
                        let mut out = handle.lock();
                        out.write_all(&output)?;
                        out.flush()?;
                    }
                    Err(x) => break Err(x),
                }
            }
        });
        let stderr_task = tokio::spawn(async move {
            let handle = io::stderr();
            loop {
                match $stderr.read().await {
                    Ok(output) => {
                        let mut out = handle.lock();
                        out.write_all(&output)?;
                        out.flush()?;
                    }
                    Err(x) => break Err(x),
                }
            }
        });

        RemoteProcessLink {
            _stdin_thread: stdin_thread,
            stdin_task,
            stdout_task,
            stderr_task,
        }
    }};
}

impl RemoteProcessLink {
    /// Creates a new process link from the pipes of a remote process
    pub fn from_remote_pipes(
        stdin: Option<RemoteStdin>,
        mut stdout: RemoteStdout,
        mut stderr: RemoteStderr,
    ) -> Self {
        from_pipes!(stdin, stdout, stderr)
    }

    /// Creates a new process link from the pipes of a remote LSP server process
    pub fn from_remote_lsp_pipes(
        stdin: Option<RemoteLspStdin>,
        mut stdout: RemoteLspStdout,
        mut stderr: RemoteLspStderr,
    ) -> Self {
        from_pipes!(stdin, stdout, stderr)
    }

    /// Shuts down the link, aborting any running tasks, and swallowing join errors
    pub async fn shutdown(self) {
        self.abort();
        let _ = self.wait().await;
    }

    /// Waits for the stdin, stdout, and stderr tasks to complete
    pub async fn wait(self) -> Result<(), JoinError> {
        if let Some(stdin_task) = self.stdin_task {
            tokio::try_join!(stdin_task, self.stdout_task, self.stderr_task).map(|_| ())
        } else {
            tokio::try_join!(self.stdout_task, self.stderr_task).map(|_| ())
        }
    }

    /// Aborts the link by aborting tasks processing stdin, stdout, and stderr
    pub fn abort(&self) {
        if let Some(stdin_task) = self.stdin_task.as_ref() {
            stdin_task.abort();
        }
        self.stdout_task.abort();
        self.stderr_task.abort();
    }
}
