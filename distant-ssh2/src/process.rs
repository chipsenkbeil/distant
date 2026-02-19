use std::future::Future;
use std::io;
use std::sync::Arc;

use distant_core::net::server::Reply;
use distant_core::protocol::{Environment, ProcessId, PtySize, Response};
use russh::client::Handle;
use russh::ChannelMsg;
use tokio::sync::mpsc;

use crate::ClientHandler;

/// Represents a spawned process
pub struct Process {
    pub id: ProcessId,
    pub stdin_tx: Option<mpsc::Sender<Vec<u8>>>,
    pub kill_tx: Option<mpsc::Sender<()>>,
    pub resize_tx: Option<mpsc::Sender<PtySize>>,
}

/// Result of spawning a process
pub struct SpawnResult {
    pub id: ProcessId,
    pub stdin: mpsc::Sender<Vec<u8>>,
    pub killer: mpsc::Sender<()>,
    pub resizer: mpsc::Sender<PtySize>,
}

/// Spawns a simple (non-PTY) process
pub async fn spawn_simple<F, Fut>(
    handle: &Handle<ClientHandler>,
    cmd: &str,
    environment: Environment,
    current_dir: Option<std::path::PathBuf>,
    reply: Box<dyn Reply<Data = Response>>,
    cleanup: F,
) -> io::Result<SpawnResult>
where
    F: FnOnce(ProcessId) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    if current_dir.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "current_dir is not supported for SSH process spawning",
        ));
    }

    // Open a channel for command execution
    let channel = handle
        .channel_open_session()
        .await
        .map_err(io::Error::other)?;

    // Set environment variables before executing the command
    for (key, value) in environment.iter() {
        // set_env may fail if the server rejects it (AcceptEnv), but we ignore failures
        let _ = channel.set_env(true, key, value).await;
    }

    // Execute the command via SSH channel
    channel.exec(true, cmd).await.map_err(io::Error::other)?;

    let id = rand::random();

    // Create channels for stdin, stdout, stderr, and process control
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);
    let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);

    // Split channel for concurrent read/write
    let (mut read_half, write_half) = channel.split();

    // Shared state to track if process was killed
    let was_killed = Arc::new(tokio::sync::Mutex::new(false));
    let was_killed_clone = was_killed.clone();

    // Spawn task to handle stdout and stderr via ChannelMsg
    let stdout_reply = reply.clone_reply();
    let stderr_reply = reply.clone_reply();
    let exit_reply = reply.clone_reply();
    let msg_id = id;
    tokio::spawn(async move {
        let mut exit_status: Option<u32> = None;
        let mut _got_eof = false;

        while let Some(msg) = read_half.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => {
                    let _ = stdout_reply.send(Response::ProcStdout {
                        id: msg_id,
                        data: data.to_vec(),
                    });
                }
                ChannelMsg::ExtendedData { ref data, ext } => {
                    if ext == 1 {
                        let _ = stderr_reply.send(Response::ProcStderr {
                            id: msg_id,
                            data: data.to_vec(),
                        });
                    }
                }
                ChannelMsg::Eof => {
                    _got_eof = true;
                }
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => {
                    exit_status = Some(status);
                }
                _ => {}
            }
        }

        // Send final exit status
        let killed = *was_killed_clone.lock().await;
        let _ = exit_reply.send(Response::ProcDone {
            id: msg_id,
            success: !killed && exit_status.map(|s| s == 0).unwrap_or(false),
            code: exit_status.map(|s| s as i32),
        });

        // Run cleanup to remove process from tracking
        cleanup(msg_id).await;
    });

    // Spawn task to handle stdin and kill signals
    let write_half = write_half;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(data) = stdin_rx.recv() => {
                    use std::io::Cursor;
                    if write_half.data(Cursor::new(data)).await.is_err() {
                        break;
                    }
                }
                Some(()) = kill_rx.recv() => {
                    *was_killed.lock().await = true;
                    let _ = write_half.eof().await;
                    break;
                }
                else => break,
            }
        }
    });

    // Create a resizer channel (not used for non-PTY processes)
    let (resize_tx, _resize_rx) = mpsc::channel(1);

    Ok(SpawnResult {
        id,
        stdin: stdin_tx,
        killer: kill_tx,
        resizer: resize_tx,
    })
}

/// Spawns a PTY process
pub async fn spawn_pty<F, Fut>(
    handle: &Handle<ClientHandler>,
    cmd: &str,
    environment: Environment,
    current_dir: Option<std::path::PathBuf>,
    size: PtySize,
    reply: Box<dyn Reply<Data = Response>>,
    cleanup: F,
) -> io::Result<SpawnResult>
where
    F: FnOnce(ProcessId) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    if current_dir.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "current_dir is not supported for SSH process spawning",
        ));
    }

    // Open a channel for PTY
    let channel = handle
        .channel_open_session()
        .await
        .map_err(io::Error::other)?;

    // Set environment variables before requesting PTY
    // Extract TERM for PTY request, but still pass all env vars via set_env
    let term_type = environment
        .get("TERM")
        .map(|s| s.as_str().to_string())
        .unwrap_or_else(|| "xterm-256color".to_string());

    for (key, value) in environment.iter() {
        let _ = channel.set_env(true, key, value).await;
    }

    // Request PTY with specified size
    channel
        .request_pty(
            true,
            &term_type,
            size.cols as u32,
            size.rows as u32,
            size.pixel_width as u32,
            size.pixel_height as u32,
            &[], // No terminal modes for now
        )
        .await
        .map_err(io::Error::other)?;

    // Run the command (or request a shell if cmd is empty)
    if cmd.is_empty() {
        channel
            .request_shell(true)
            .await
            .map_err(io::Error::other)?;
    } else {
        channel.exec(true, cmd).await.map_err(io::Error::other)?;
    }

    let id = rand::random();

    // Create channels for stdin, stdout (PTY combines stdout/stderr), and process control
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);
    let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
    let (resize_tx, mut resize_rx) = mpsc::channel::<PtySize>(1);

    // Split channel for concurrent read/write
    let (mut read_half, write_half) = channel.split();

    // Shared state to track if process was killed
    let was_killed = Arc::new(tokio::sync::Mutex::new(false));
    let was_killed_clone = was_killed.clone();

    // Spawn task to handle PTY output (stdout/stderr combined) via ChannelMsg
    let stdout_reply = reply.clone_reply();
    let exit_reply = reply.clone_reply();
    let msg_id = id;
    tokio::spawn(async move {
        let mut exit_status: Option<u32> = None;
        let mut _got_eof = false;

        while let Some(msg) = read_half.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => {
                    let _ = stdout_reply.send(Response::ProcStdout {
                        id: msg_id,
                        data: data.to_vec(),
                    });
                }
                ChannelMsg::Eof => {
                    _got_eof = true;
                }
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => {
                    exit_status = Some(status);
                }
                _ => {}
            }
        }

        // Send final exit status
        let killed = *was_killed_clone.lock().await;
        let _ = exit_reply.send(Response::ProcDone {
            id: msg_id,
            success: !killed && exit_status.map(|s| s == 0).unwrap_or(false),
            code: exit_status.map(|s| s as i32),
        });

        // Run cleanup to remove process from tracking
        cleanup(msg_id).await;
    });

    // Spawn task to handle stdin, kill signals, and PTY resize
    let write_half = write_half;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(data) = stdin_rx.recv() => {
                    use std::io::Cursor;
                    if write_half.data(Cursor::new(data)).await.is_err() {
                        break;
                    }
                }
                Some(()) = kill_rx.recv() => {
                    *was_killed.lock().await = true;
                    let _ = write_half.eof().await;
                    break;
                }
                Some(new_size) = resize_rx.recv() => {
                    if write_half.window_change(
                        new_size.cols as u32,
                        new_size.rows as u32,
                        new_size.pixel_width as u32,
                        new_size.pixel_height as u32,
                    ).await.is_err() {
                        break;
                    }
                }
                else => break,
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
