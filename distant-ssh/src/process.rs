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

#[cfg(test)]
mod tests {
    use super::*;

    // --- Process struct tests ---

    #[test]
    fn process_struct_construction_with_all_channels() {
        let (stdin_tx, _stdin_rx) = mpsc::channel::<Vec<u8>>(32);
        let (kill_tx, _kill_rx) = mpsc::channel::<()>(1);
        let (resize_tx, _resize_rx) = mpsc::channel::<PtySize>(1);

        let process = Process {
            id: 42,
            stdin_tx: Some(stdin_tx),
            kill_tx: Some(kill_tx),
            resize_tx: Some(resize_tx),
        };

        assert_eq!(process.id, 42);
        assert!(process.stdin_tx.is_some());
        assert!(process.kill_tx.is_some());
        assert!(process.resize_tx.is_some());
    }

    #[test]
    fn process_struct_construction_with_no_channels() {
        let process = Process {
            id: 0,
            stdin_tx: None,
            kill_tx: None,
            resize_tx: None,
        };

        assert_eq!(process.id, 0);
        assert!(process.stdin_tx.is_none());
        assert!(process.kill_tx.is_none());
        assert!(process.resize_tx.is_none());
    }

    #[test]
    fn process_struct_partial_channels() {
        let (stdin_tx, _stdin_rx) = mpsc::channel::<Vec<u8>>(1);

        let process = Process {
            id: 99,
            stdin_tx: Some(stdin_tx),
            kill_tx: None,
            resize_tx: None,
        };

        assert_eq!(process.id, 99);
        assert!(process.stdin_tx.is_some());
        assert!(process.kill_tx.is_none());
        assert!(process.resize_tx.is_none());
    }

    #[test]
    fn process_struct_take_channels() {
        let (stdin_tx, _stdin_rx) = mpsc::channel::<Vec<u8>>(1);
        let (kill_tx, _kill_rx) = mpsc::channel::<()>(1);
        let (resize_tx, _resize_rx) = mpsc::channel::<PtySize>(1);

        let mut process = Process {
            id: 7,
            stdin_tx: Some(stdin_tx),
            kill_tx: Some(kill_tx),
            resize_tx: Some(resize_tx),
        };

        // Taking channels should leave None behind
        let taken_stdin = process.stdin_tx.take();
        assert!(taken_stdin.is_some());
        assert!(process.stdin_tx.is_none());

        let taken_kill = process.kill_tx.take();
        assert!(taken_kill.is_some());
        assert!(process.kill_tx.is_none());

        let taken_resize = process.resize_tx.take();
        assert!(taken_resize.is_some());
        assert!(process.resize_tx.is_none());
    }

    #[test_log::test(tokio::test)]
    async fn process_stdin_channel_send_receive() {
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);

        let process = Process {
            id: 1,
            stdin_tx: Some(stdin_tx),
            kill_tx: None,
            resize_tx: None,
        };

        // Send data through the stdin channel
        let sender = process.stdin_tx.as_ref().unwrap();
        sender.send(b"hello".to_vec()).await.unwrap();

        let received = stdin_rx.recv().await.unwrap();
        assert_eq!(received, b"hello");
    }

    #[test_log::test(tokio::test)]
    async fn process_kill_channel_send_receive() {
        let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);

        let process = Process {
            id: 2,
            stdin_tx: None,
            kill_tx: Some(kill_tx),
            resize_tx: None,
        };

        let killer = process.kill_tx.as_ref().unwrap();
        killer.send(()).await.unwrap();

        let received = kill_rx.recv().await;
        assert!(received.is_some());
    }

    #[test_log::test(tokio::test)]
    async fn process_resize_channel_send_receive() {
        let (resize_tx, mut resize_rx) = mpsc::channel::<PtySize>(1);

        let process = Process {
            id: 3,
            stdin_tx: None,
            kill_tx: None,
            resize_tx: Some(resize_tx),
        };

        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };

        let resizer = process.resize_tx.as_ref().unwrap();
        resizer.send(size).await.unwrap();

        let received = resize_rx.recv().await.unwrap();
        assert_eq!(received.rows, 24);
        assert_eq!(received.cols, 80);
    }

    // --- SpawnResult struct tests ---

    #[test]
    fn spawn_result_struct_construction() {
        let (stdin_tx, _stdin_rx) = mpsc::channel::<Vec<u8>>(32);
        let (kill_tx, _kill_rx) = mpsc::channel::<()>(1);
        let (resize_tx, _resize_rx) = mpsc::channel::<PtySize>(1);

        let result = SpawnResult {
            id: 100,
            stdin: stdin_tx,
            killer: kill_tx,
            resizer: resize_tx,
        };

        assert_eq!(result.id, 100);
    }

    #[test_log::test(tokio::test)]
    async fn spawn_result_channels_are_functional() {
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);
        let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
        let (resize_tx, mut resize_rx) = mpsc::channel::<PtySize>(1);

        let result = SpawnResult {
            id: 200,
            stdin: stdin_tx,
            killer: kill_tx,
            resizer: resize_tx,
        };

        // Test stdin
        result.stdin.send(b"input data".to_vec()).await.unwrap();
        let received = stdin_rx.recv().await.unwrap();
        assert_eq!(received, b"input data");

        // Test kill
        result.killer.send(()).await.unwrap();
        assert!(kill_rx.recv().await.is_some());

        // Test resize
        let size = PtySize {
            rows: 50,
            cols: 120,
            pixel_width: 800,
            pixel_height: 600,
        };
        result.resizer.send(size).await.unwrap();
        let received_size = resize_rx.recv().await.unwrap();
        assert_eq!(received_size.rows, 50);
        assert_eq!(received_size.cols, 120);
        assert_eq!(received_size.pixel_width, 800);
        assert_eq!(received_size.pixel_height, 600);
    }

    #[test]
    fn spawn_result_with_max_process_id() {
        let (stdin_tx, _) = mpsc::channel::<Vec<u8>>(1);
        let (kill_tx, _) = mpsc::channel::<()>(1);
        let (resize_tx, _) = mpsc::channel::<PtySize>(1);

        let result = SpawnResult {
            id: ProcessId::MAX,
            stdin: stdin_tx,
            killer: kill_tx,
            resizer: resize_tx,
        };

        assert_eq!(result.id, ProcessId::MAX);
    }

    #[test]
    fn spawn_result_with_zero_process_id() {
        let (stdin_tx, _) = mpsc::channel::<Vec<u8>>(1);
        let (kill_tx, _) = mpsc::channel::<()>(1);
        let (resize_tx, _) = mpsc::channel::<PtySize>(1);

        let result = SpawnResult {
            id: 0,
            stdin: stdin_tx,
            killer: kill_tx,
            resizer: resize_tx,
        };

        assert_eq!(result.id, 0);
    }

    #[test_log::test(tokio::test)]
    async fn process_stdin_channel_multiple_sends() {
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);

        let process = Process {
            id: 10,
            stdin_tx: Some(stdin_tx),
            kill_tx: None,
            resize_tx: None,
        };

        let sender = process.stdin_tx.as_ref().unwrap();
        sender.send(b"first".to_vec()).await.unwrap();
        sender.send(b"second".to_vec()).await.unwrap();
        sender.send(b"third".to_vec()).await.unwrap();

        assert_eq!(stdin_rx.recv().await.unwrap(), b"first");
        assert_eq!(stdin_rx.recv().await.unwrap(), b"second");
        assert_eq!(stdin_rx.recv().await.unwrap(), b"third");
    }

    #[test_log::test(tokio::test)]
    async fn process_stdin_channel_empty_data() {
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);

        let process = Process {
            id: 11,
            stdin_tx: Some(stdin_tx),
            kill_tx: None,
            resize_tx: None,
        };

        let sender = process.stdin_tx.as_ref().unwrap();
        sender.send(vec![]).await.unwrap();

        let received = stdin_rx.recv().await.unwrap();
        assert!(received.is_empty());
    }

    #[test_log::test(tokio::test)]
    async fn process_stdin_channel_large_data() {
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);

        let process = Process {
            id: 12,
            stdin_tx: Some(stdin_tx),
            kill_tx: None,
            resize_tx: None,
        };

        let large_data = vec![0xAA; 65536]; // 64KB
        let sender = process.stdin_tx.as_ref().unwrap();
        sender.send(large_data.clone()).await.unwrap();

        let received = stdin_rx.recv().await.unwrap();
        assert_eq!(received.len(), 65536);
        assert_eq!(received, large_data);
    }

    #[test_log::test(tokio::test)]
    async fn process_resize_channel_different_sizes() {
        let (resize_tx, mut resize_rx) = mpsc::channel::<PtySize>(4);

        let process = Process {
            id: 13,
            stdin_tx: None,
            kill_tx: None,
            resize_tx: Some(resize_tx),
        };

        let resizer = process.resize_tx.as_ref().unwrap();

        // Send various sizes
        let sizes = vec![
            PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            PtySize {
                rows: 50,
                cols: 120,
                pixel_width: 800,
                pixel_height: 600,
            },
            PtySize {
                rows: 1,
                cols: 1,
                pixel_width: 1,
                pixel_height: 1,
            },
        ];

        for size in &sizes {
            resizer.send(*size).await.unwrap();
        }

        for expected in &sizes {
            let received = resize_rx.recv().await.unwrap();
            assert_eq!(received.rows, expected.rows);
            assert_eq!(received.cols, expected.cols);
            assert_eq!(received.pixel_width, expected.pixel_width);
            assert_eq!(received.pixel_height, expected.pixel_height);
        }
    }

    #[test_log::test(tokio::test)]
    async fn process_dropped_sender_closes_channel() {
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(1);

        {
            let _process = Process {
                id: 14,
                stdin_tx: Some(stdin_tx),
                kill_tx: None,
                resize_tx: None,
            };
            // _process (and stdin_tx) dropped here
        }

        // Receiving should return None since the sender was dropped
        assert!(stdin_rx.recv().await.is_none());
    }
}
