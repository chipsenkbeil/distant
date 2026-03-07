use std::io::{self, Write};
use std::thread;
use std::time::Duration;

use distant_core::{
    RemoteLspStderr, RemoteLspStdin, RemoteLspStdout, RemoteStderr, RemoteStdin, RemoteStdout,
};
use log::*;
use tokio::task::JoinHandle;

use super::stdin;

/// Maximum time to wait for stdout/stderr to drain after process exit.
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Represents a link between a remote process' stdin/stdout/stderr and this process'
/// stdin/stdout/stderr
pub struct RemoteProcessLink {
    _stdin_thread: Option<thread::JoinHandle<()>>,
    stdin_task: Option<JoinHandle<io::Result<()>>>,
    stdout_task: JoinHandle<io::Result<()>>,
    stderr_task: JoinHandle<io::Result<()>>,
}

macro_rules! from_pipes {
    ($stdin:expr, $stdout:expr, $stderr:expr, $buffer:expr, $stdout_filter:expr) => {{
        let mut stdin_thread = None;
        let mut stdin_task = None;
        if let Some(mut stdin_handle) = $stdin {
            let (thread, mut rx) = stdin::spawn_channel($buffer);
            let task = tokio::spawn(async move {
                loop {
                    if let Some(input) = rx.recv().await {
                        trace!("Forwarding stdin: {:?}", String::from_utf8_lossy(&input));
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
        let stdout_filter: Option<fn(&[u8], &mut Vec<u8>)> = $stdout_filter;
        let stdout_task = tokio::spawn(async move {
            let handle = io::stdout();
            let mut filtered_buf = if stdout_filter.is_some() {
                Vec::with_capacity(8192)
            } else {
                Vec::new()
            };
            loop {
                match $stdout.read().await {
                    Ok(output) => {
                        trace!(
                            "stdout: {} bytes: {:02x?}",
                            output.len(),
                            &output[..output.len().min(64)]
                        );
                        if let Some(filter) = stdout_filter {
                            filtered_buf.clear();
                            filter(&output, &mut filtered_buf);
                            if !filtered_buf.is_empty() {
                                let mut out = handle.lock();
                                out.write_all(&filtered_buf)?;
                                out.flush()?;
                            }
                        } else {
                            let mut out = handle.lock();
                            out.write_all(&output)?;
                            out.flush()?;
                        }
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
                        trace!(
                            "stderr: {} bytes: {:02x?}",
                            output.len(),
                            &output[..output.len().min(64)]
                        );
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
    /// Creates a new process link from the pipes of a remote process.
    ///
    /// `max_pipe_chunk_size` represents the maximum size (in bytes) of data that will be read from
    /// stdin at one time to forward to the remote process.
    pub fn from_remote_pipes(
        stdin: Option<RemoteStdin>,
        mut stdout: RemoteStdout,
        mut stderr: RemoteStderr,
        max_pipe_chunk_size: usize,
    ) -> Self {
        from_pipes!(stdin, stdout, stderr, max_pipe_chunk_size, None)
    }

    /// Creates a new process link from the pipes of a remote process, applying a filter to stdout.
    ///
    /// `stdout_filter` is called on each stdout chunk before writing to local stdout. The filter
    /// receives the raw bytes and must write the filtered output into the provided `Vec`. If the
    /// filter produces empty output, the write is skipped entirely.
    ///
    /// `max_pipe_chunk_size` represents the maximum size (in bytes) of data that will be read from
    /// stdin at one time to forward to the remote process.
    pub fn from_remote_pipes_filtered(
        stdin: Option<RemoteStdin>,
        mut stdout: RemoteStdout,
        mut stderr: RemoteStderr,
        max_pipe_chunk_size: usize,
        stdout_filter: fn(&[u8], &mut Vec<u8>),
    ) -> Self {
        from_pipes!(
            stdin,
            stdout,
            stderr,
            max_pipe_chunk_size,
            Some(stdout_filter)
        )
    }

    /// Creates a new process link from the pipes of a remote LSP server process.
    ///
    /// `max_pipe_chunk_size` represents the maximum size (in bytes) of data that will be read from
    /// stdin at one time to forward to the remote process.
    pub fn from_remote_lsp_pipes(
        stdin: Option<RemoteLspStdin>,
        mut stdout: RemoteLspStdout,
        mut stderr: RemoteLspStderr,
        max_pipe_chunk_size: usize,
    ) -> Self {
        from_pipes!(stdin, stdout, stderr, max_pipe_chunk_size, None)
    }

    /// Shuts down the link, letting stdout/stderr drain before returning.
    ///
    /// Stdin is aborted immediately (nothing to drain). Stdout and stderr tasks
    /// are allowed to finish naturally — they terminate when the remote process
    /// exits and the mpsc channel senders are dropped, causing `recv()` to
    /// return `None` (mapped to `BrokenPipe`). A timeout acts as a safety net.
    pub async fn shutdown(self) {
        // Abort stdin — we don't need to drain input
        if let Some(stdin_task) = self.stdin_task {
            stdin_task.abort();
            let _ = stdin_task.await;
        }

        // Let stdout/stderr drain pending data before returning.
        // They'll exit once their mpsc senders are dropped (on ProcDone).
        let drain = async {
            let _ = self.stdout_task.await;
            let _ = self.stderr_task.await;
        };
        if tokio::time::timeout(OUTPUT_DRAIN_TIMEOUT, drain)
            .await
            .is_err()
        {
            warn!(
                "stdout/stderr drain timed out after {}s",
                OUTPUT_DRAIN_TIMEOUT.as_secs()
            );
        }
    }
}
