use std::future::Future;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use async_compat::CompatExt;
use distant_core::net::server::Reply;
use distant_core::protocol::{Environment, ProcessId, PtySize, Response};
use log::*;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use wezterm_ssh::{
    Child, ChildKiller, ExecResult, MasterPty, PtySize as PortablePtySize, Session, SshChildProcess,
};

const MAX_PIPE_CHUNK_SIZE: usize = 8192;
const THREAD_PAUSE_MILLIS: u64 = 1;

/// Result of spawning a process, containing means to send stdin, means to kill the process,
/// and the initialization function to use to start processing stdin, stdout, and stderr
pub struct SpawnResult {
    pub id: ProcessId,
    pub stdin: mpsc::Sender<Vec<u8>>,
    pub killer: mpsc::Sender<()>,
    pub resizer: mpsc::Sender<PtySize>,
}

/// Spawns a non-pty process, returning a function that initializes processing
/// stdin, stdout, and stderr once called (for lazy processing)
pub async fn spawn_simple<F, R>(
    session: &Session,
    cmd: &str,
    environment: Environment,
    current_dir: Option<PathBuf>,
    reply: Box<dyn Reply<Data = Response>>,
    cleanup: F,
) -> io::Result<SpawnResult>
where
    F: FnOnce(ProcessId) -> R + Send + 'static,
    R: Future<Output = ()> + Send + 'static,
{
    if current_dir.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "current_dir is not supported",
        ));
    }

    let ExecResult {
        mut stdin,
        mut stdout,
        mut stderr,
        mut child,
    } = session
        .exec(
            cmd,
            if environment.is_empty() {
                None
            } else {
                Some(environment)
            },
        )
        .compat()
        .await
        .map_err(to_other_error)?;

    // Update to be nonblocking for reading and writing
    stdin.set_non_blocking(true).map_err(to_other_error)?;
    stdout.set_non_blocking(true).map_err(to_other_error)?;
    stderr.set_non_blocking(true).map_err(to_other_error)?;

    // Check if the process died immediately and report
    // an error if that's the case
    if let Ok(Some(exit_status)) = child.try_wait() {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("Process exited early: {exit_status:?}"),
        ));
    }

    let (stdin_tx, stdin_rx) = mpsc::channel(1);
    let (kill_tx, kill_rx) = mpsc::channel(1);

    let id = rand::random();
    let session = session.clone();
    let stdout_task = spawn_nonblocking_stdout_task(id, stdout, reply.clone_reply());
    let stderr_task = spawn_nonblocking_stderr_task(id, stderr, reply.clone_reply());
    let stdin_task = spawn_nonblocking_stdin_task(id, stdin, stdin_rx);
    drop(spawn_cleanup_task(
        session,
        id,
        child,
        kill_rx,
        stdin_task,
        stdout_task,
        Some(stderr_task),
        reply,
        cleanup,
    ));

    // Create a resizer that is already closed since a simple process does not resize
    let resizer = mpsc::channel(1).0;

    Ok(SpawnResult {
        id,
        stdin: stdin_tx,
        killer: kill_tx,
        resizer,
    })
}

/// Spawns a pty process, returning a function that initializes processing
/// stdin and stdout/stderr once called (for lazy processing)
pub async fn spawn_pty<F, R>(
    session: &Session,
    cmd: &str,
    environment: Environment,
    current_dir: Option<PathBuf>,
    size: PtySize,
    reply: Box<dyn Reply<Data = Response>>,
    cleanup: F,
) -> io::Result<SpawnResult>
where
    F: FnOnce(ProcessId) -> R + Send + 'static,
    R: Future<Output = ()> + Send + 'static,
{
    if current_dir.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "current_dir is not supported",
        ));
    }

    let term = environment
        .get("TERM")
        .map(ToString::to_string)
        .unwrap_or_else(|| String::from("xterm-256color"));
    let (pty, mut child) = session
        .request_pty(
            &term,
            to_portable_size(size),
            Some(cmd),
            if environment.is_empty() {
                None
            } else {
                Some(environment)
            },
        )
        .compat()
        .await
        .map_err(to_other_error)?;

    // Check if the process died immediately and report
    // an error if that's the case
    if let Ok(Some(exit_status)) = child.try_wait() {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("Process exited early: {exit_status:?}"),
        ));
    }

    let reader = pty
        .try_clone_reader()
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
    let writer = pty
        .try_clone_writer()
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

    let (stdin_tx, stdin_rx) = mpsc::channel(1);
    let (kill_tx, kill_rx) = mpsc::channel(1);

    let id = rand::random();
    let session = session.clone();
    let stdout_task = spawn_blocking_stdout_task(id, reader, reply.clone_reply());
    let stdin_task = spawn_blocking_stdin_task(id, writer, stdin_rx);
    drop(spawn_cleanup_task(
        session,
        id,
        child,
        kill_rx,
        stdin_task,
        stdout_task,
        None,
        reply,
        cleanup,
    ));

    let (resize_tx, mut resize_rx) = mpsc::channel::<PtySize>(1);
    tokio::spawn(async move {
        while let Some(size) = resize_rx.recv().await {
            if pty.resize(to_portable_size(size)).is_err() {
                break;
            }
        }
    });

    Ok(SpawnResult {
        id,
        stdin: stdin_tx,
        killer: kill_tx,
        resizer: resize_tx,
    })
}

fn spawn_blocking_stdout_task(
    id: ProcessId,
    mut reader: impl Read + Send + 'static,
    reply: Box<dyn Reply<Data = Response>>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
        loop {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let payload = Response::ProcStdout {
                        id,
                        data: buf[..n].to_vec(),
                    };
                    if reply.send(payload).is_err() {
                        error!("[Ssh | Proc {}] Stdout channel closed", id);
                        break;
                    }

                    std::thread::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS));
                }
                Ok(_) => break,
                Err(x) => {
                    error!("[Ssh | Proc {}] Stdout unexpectedly closed: {}", id, x);
                    break;
                }
            }
        }
    })
}

fn spawn_nonblocking_stdout_task(
    id: ProcessId,
    mut reader: impl Read + Send + 'static,
    reply: Box<dyn Reply<Data = Response>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
        loop {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let payload = Response::ProcStdout {
                        id,
                        data: buf[..n].to_vec(),
                    };
                    if reply.send(payload).is_err() {
                        error!("[Ssh | Proc {}] Stdout channel closed", id);
                        break;
                    }

                    tokio::time::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS)).await;
                }
                Ok(_) => break,
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS)).await;
                }
                Err(x) => {
                    error!("[Ssh | Proc {}] Stdout unexpectedly closed: {}", id, x);
                    break;
                }
            }
        }
    })
}

fn spawn_nonblocking_stderr_task(
    id: ProcessId,
    mut reader: impl Read + Send + 'static,
    reply: Box<dyn Reply<Data = Response>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
        loop {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let payload = Response::ProcStderr {
                        id,
                        data: buf[..n].to_vec(),
                    };
                    if reply.send(payload).is_err() {
                        error!("[Ssh | Proc {}] Stderr channel closed", id);
                        break;
                    }

                    tokio::time::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS)).await;
                }
                Ok(_) => break,
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS)).await;
                }
                Err(x) => {
                    error!("[Ssh | Proc {}] Stderr unexpectedly closed: {}", id, x);
                    break;
                }
            }
        }
    })
}

fn spawn_blocking_stdin_task(
    id: ProcessId,
    mut writer: impl Write + Send + 'static,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        while let Some(data) = rx.blocking_recv() {
            if let Err(x) = writer.write_all(&data) {
                error!("[Ssh | Proc {}] Failed to send stdin: {}", id, x);
                break;
            }

            std::thread::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS));
        }
    })
}

fn spawn_nonblocking_stdin_task(
    id: ProcessId,
    mut writer: impl Write + Send + 'static,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            if let Err(x) = writer.write_all(&data) {
                // In non-blocking mode, we'll just pause and try again if
                // the IO would block here; otherwise, stop the task
                if x.kind() != io::ErrorKind::WouldBlock {
                    error!("[Ssh | Proc {}] Failed to send stdin: {}", id, x);
                    break;
                }
            }

            tokio::time::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS)).await;
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_cleanup_task<F, R>(
    session: Session,
    id: ProcessId,
    mut child: SshChildProcess,
    mut kill_rx: mpsc::Receiver<()>,
    stdin_task: JoinHandle<()>,
    stdout_task: JoinHandle<()>,
    stderr_task: Option<JoinHandle<()>>,
    reply: Box<dyn Reply<Data = Response>>,
    cleanup: F,
) -> JoinHandle<()>
where
    F: FnOnce(ProcessId) -> R + Send + 'static,
    R: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let mut should_kill = false;
        let mut success = false;
        tokio::select! {
            _ = kill_rx.recv() => {
                should_kill = true;
            }
            result = child.async_wait().compat() => {
                match result {
                    Ok(status) => {
                        success = status.success();
                    }
                    Err(x) => {
                        error!("[Ssh | Proc {}] Waiting on process failed: {}", id, x);
                    }
                }
            }
        }

        // Force stdin task to abort if it hasn't exited as there is no
        // point to sending any more stdin
        stdin_task.abort();

        if should_kill {
            debug!("[Ssh | Proc {}] Killing", id);

            if let Err(x) = child.kill() {
                error!("[Ssh | Proc {}] Unable to kill process: {}", id, x);
            }

            // NOTE: At the moment, child.kill does nothing for wezterm_ssh::SshChildProcess;
            //       so, we need to manually run kill/taskkill to make sure that the
            //       process is sent a kill signal
            if let Some(pid) = child.process_id() {
                let _ = session.exec(&format!("kill -9 {pid}"), None).compat().await;
                let _ = session
                    .exec(&format!("taskkill /F /PID {pid}"), None)
                    .compat()
                    .await;
            }
        } else {
            debug!(
                "[Ssh | Proc {}] Completed and waiting on stdout & stderr tasks",
                id
            );
        }

        // We're done with the child, so drop it
        drop(child);

        if let Some(task) = stderr_task {
            if let Err(x) = task.await {
                error!("[Ssh | Proc {}] Join on stderr task failed: {}", id, x);
            }
        }

        if let Err(x) = stdout_task.await {
            error!("[Ssh | Proc {}] Join on stdout task failed: {}", id, x);
        }

        cleanup(id).await;

        let payload = Response::ProcDone {
            id,
            success: !should_kill && success,
            code: if success { Some(0) } else { None },
        };

        if reply.send(payload).is_err() {
            error!("[Ssh | Proc {}] Failed to send done", id,);
        }
    })
}

fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::Other, err)
}

fn to_portable_size(size: PtySize) -> PortablePtySize {
    PortablePtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: size.pixel_width,
        pixel_height: size.pixel_height,
    }
}
