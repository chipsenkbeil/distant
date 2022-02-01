use async_compat::CompatExt;
use distant_core::{PtySize, ResponseData};
use log::*;
use std::{
    future::Future,
    io::{self, Read, Write},
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};
use wezterm_ssh::{
    Child, ChildKiller, ExecResult, MasterPty, PtySize as PortablePtySize, Session, SshChildProcess,
};

const MAX_PIPE_CHUNK_SIZE: usize = 8192;
const THREAD_PAUSE_MILLIS: u64 = 50;

/// Result of spawning a process, containing means to send stdin, means to kill the process,
/// and the initialization function to use to start processing stdin, stdout, and stderr
pub struct SpawnResult {
    pub id: usize,
    pub stdin: mpsc::Sender<Vec<u8>>,
    pub killer: mpsc::Sender<()>,
    pub resizer: mpsc::Sender<PtySize>,
    pub initialize: Box<dyn FnOnce(mpsc::Sender<Vec<ResponseData>>) + Send>,
}

/// Spawns a non-pty process, returning a function that initializes processing
/// stdin, stdout, and stderr once called (for lazy processing)
pub async fn spawn_simple<F, R>(session: &Session, cmd: &str, cleanup: F) -> io::Result<SpawnResult>
where
    F: FnOnce(usize) -> R + Send + 'static,
    R: Future<Output = ()> + Send + 'static,
{
    let ExecResult {
        mut stdin,
        mut stdout,
        mut stderr,
        mut child,
    } = session
        .exec(cmd, None)
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
            format!("Process exited early: {:?}", exit_status),
        ));
    }

    let (stdin_tx, stdin_rx) = mpsc::channel(1);
    let (kill_tx, kill_rx) = mpsc::channel(1);

    let id = rand::random();
    let session = session.clone();
    let initialize = Box::new(move |reply: mpsc::Sender<Vec<ResponseData>>| {
        let stdout_task = spawn_nonblocking_stdout_task(id, stdout, reply.clone());
        let stderr_task = spawn_nonblocking_stderr_task(id, stderr, reply.clone());
        let stdin_task = spawn_nonblocking_stdin_task(id, stdin, stdin_rx);
        let _ = spawn_cleanup_task(
            session,
            id,
            child,
            kill_rx,
            stdin_task,
            stdout_task,
            Some(stderr_task),
            reply,
            cleanup,
        );
    });

    // Create a resizer that is already closed since a simple process does not resize
    let resizer = mpsc::channel(1).0;

    Ok(SpawnResult {
        id,
        stdin: stdin_tx,
        killer: kill_tx,
        resizer,
        initialize,
    })
}

/// Spawns a pty process, returning a function that initializes processing
/// stdin and stdout/stderr once called (for lazy processing)
pub async fn spawn_pty<F, R>(
    session: &Session,
    cmd: &str,
    size: PtySize,
    cleanup: F,
) -> io::Result<SpawnResult>
where
    F: FnOnce(usize) -> R + Send + 'static,
    R: Future<Output = ()> + Send + 'static,
{
    // TODO: Do we need to support other terminal types for TERM?
    let (pty, mut child) = session
        .request_pty("xterm-256color", to_portable_size(size), Some(cmd), None)
        .compat()
        .await
        .map_err(to_other_error)?;

    // Check if the process died immediately and report
    // an error if that's the case
    if let Ok(Some(exit_status)) = child.try_wait() {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("Process exited early: {:?}", exit_status),
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
    let initialize = Box::new(move |reply: mpsc::Sender<Vec<ResponseData>>| {
        let stdout_task = spawn_blocking_stdout_task(id, reader, reply.clone());
        let stdin_task = spawn_blocking_stdin_task(id, writer, stdin_rx);
        let _ = spawn_cleanup_task(
            session,
            id,
            child,
            kill_rx,
            stdin_task,
            stdout_task,
            None,
            reply,
            cleanup,
        );
    });

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
        initialize,
    })
}

fn spawn_blocking_stdout_task(
    id: usize,
    mut reader: impl Read + Send + 'static,
    tx: mpsc::Sender<Vec<ResponseData>>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
        loop {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let payload = vec![ResponseData::ProcStdout {
                        id,
                        data: buf[..n].to_vec(),
                    }];
                    if tx.blocking_send(payload).is_err() {
                        error!("<Ssh | Proc {}> Stdout channel closed", id);
                        break;
                    }

                    std::thread::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS));
                }
                Ok(_) => break,
                Err(x) => {
                    error!("<Ssh | Proc {}> Stdout unexpectedly closed: {}", id, x);
                    break;
                }
            }
        }
    })
}

fn spawn_nonblocking_stdout_task(
    id: usize,
    mut reader: impl Read + Send + 'static,
    tx: mpsc::Sender<Vec<ResponseData>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
        loop {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let payload = vec![ResponseData::ProcStdout {
                        id,
                        data: buf[..n].to_vec(),
                    }];
                    if tx.send(payload).await.is_err() {
                        error!("<Ssh | Proc {}> Stdout channel closed", id);
                        break;
                    }

                    tokio::time::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS)).await;
                }
                Ok(_) => break,
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS)).await;
                }
                Err(x) => {
                    error!("<Ssh | Proc {}> Stdout unexpectedly closed: {}", id, x);
                    break;
                }
            }
        }
    })
}

fn spawn_nonblocking_stderr_task(
    id: usize,
    mut reader: impl Read + Send + 'static,
    tx: mpsc::Sender<Vec<ResponseData>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
        loop {
            match reader.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let payload = vec![ResponseData::ProcStderr {
                        id,
                        data: buf[..n].to_vec(),
                    }];
                    if tx.send(payload).await.is_err() {
                        error!("<Ssh | Proc {}> Stderr channel closed", id);
                        break;
                    }

                    tokio::time::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS)).await;
                }
                Ok(_) => break,
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                    tokio::time::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS)).await;
                }
                Err(x) => {
                    error!("<Ssh | Proc {}> Stderr unexpectedly closed: {}", id, x);
                    break;
                }
            }
        }
    })
}

fn spawn_blocking_stdin_task(
    id: usize,
    mut writer: impl Write + Send + 'static,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || {
        while let Some(data) = rx.blocking_recv() {
            if let Err(x) = writer.write_all(&data) {
                error!("<Ssh | Proc {}> Failed to send stdin: {}", id, x);
                break;
            }

            std::thread::sleep(Duration::from_millis(THREAD_PAUSE_MILLIS));
        }
    })
}

fn spawn_nonblocking_stdin_task(
    id: usize,
    mut writer: impl Write + Send + 'static,
    mut rx: mpsc::Receiver<Vec<u8>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            if let Err(x) = writer.write_all(&data) {
                // In non-blocking mode, we'll just pause and try again if
                // the IO would block here; otherwise, stop the task
                if x.kind() != io::ErrorKind::WouldBlock {
                    error!("<Ssh | Proc {}> Failed to send stdin: {}", id, x);
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
    id: usize,
    mut child: SshChildProcess,
    mut kill_rx: mpsc::Receiver<()>,
    stdin_task: JoinHandle<()>,
    stdout_task: JoinHandle<()>,
    stderr_task: Option<JoinHandle<()>>,
    tx: mpsc::Sender<Vec<ResponseData>>,
    cleanup: F,
) -> JoinHandle<()>
where
    F: FnOnce(usize) -> R + Send + 'static,
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
                        error!("<Ssh | Proc {}> Waiting on process failed: {}", id, x);
                    }
                }
            }
        }

        // Force stdin task to abort if it hasn't exited as there is no
        // point to sending any more stdin
        stdin_task.abort();

        if should_kill {
            debug!("<Ssh | Proc {}> Killing", id);

            if let Err(x) = child.kill() {
                error!("<Ssh | Proc {}> Unable to kill process: {}", id, x);
            }

            // NOTE: At the moment, child.kill does nothing for wezterm_ssh::SshChildProcess;
            //       so, we need to manually run kill/taskkill to make sure that the
            //       process is sent a kill signal
            if let Some(pid) = child.process_id() {
                let _ = session
                    .exec(&format!("kill -9 {}", pid), None)
                    .compat()
                    .await;
                let _ = session
                    .exec(&format!("taskkill /F /PID {}", pid), None)
                    .compat()
                    .await;
            }
        } else {
            debug!(
                "<Ssh | Proc {}> Completed and waiting on stdout & stderr tasks",
                id
            );
        }

        // We're done with the child, so drop it
        drop(child);

        if let Some(task) = stderr_task {
            if let Err(x) = task.await {
                error!("<Ssh | Proc {}> Join on stderr task failed: {}", id, x);
            }
        }

        if let Err(x) = stdout_task.await {
            error!("<Ssh | Proc {}> Join on stdout task failed: {}", id, x);
        }

        cleanup(id).await;

        let payload = vec![ResponseData::ProcDone {
            id,
            success: !should_kill && success,
            code: if success { Some(0) } else { None },
        }];

        if tx.send(payload).await.is_err() {
            error!("<Ssh | Proc {}> Failed to send done", id,);
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
