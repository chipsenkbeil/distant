//! Process management for Docker containers via the Docker exec API.

use std::future::Future;
use std::io;

use bollard::Docker;
use bollard::exec::{CreateExecOptions, ResizeExecOptions, StartExecOptions, StartExecResults};
use distant_core::net::server::Reply;
use distant_core::protocol::{Environment, ProcessId, PtySize, Response};
use futures::StreamExt;
use log::*;
use tokio::sync::mpsc;

use crate::DockerFamily;

/// Represents a spawned process tracked by the Docker API.
#[allow(dead_code)]
pub struct Process {
    /// Unique process identifier.
    pub id: ProcessId,

    /// Sender for stdin data. `None` after the channel is closed.
    pub stdin_tx: Option<mpsc::Sender<Vec<u8>>>,

    /// Sender to request process kill. `None` after kill is sent.
    pub kill_tx: Option<mpsc::Sender<()>>,

    /// Sender for PTY resize requests. `None` for non-PTY processes.
    pub resize_tx: Option<mpsc::Sender<PtySize>>,

    /// Docker exec ID for this process, used for kill-by-PID fallback.
    pub exec_id: String,
}

/// Result of spawning a process.
pub struct SpawnResult {
    /// Unique process identifier.
    pub id: ProcessId,

    /// Sender for stdin data.
    pub stdin: mpsc::Sender<Vec<u8>>,

    /// Sender to request process kill.
    pub killer: mpsc::Sender<()>,

    /// Sender for PTY resize requests.
    pub resizer: mpsc::Sender<PtySize>,

    /// Docker exec ID.
    pub exec_id: String,
}

/// Spawns a simple (non-PTY) process in a Docker container.
#[allow(clippy::too_many_arguments)]
pub async fn spawn_simple<F, Fut>(
    client: &Docker,
    container: &str,
    cmd: &str,
    environment: Environment,
    current_dir: Option<std::path::PathBuf>,
    family: DockerFamily,
    user: Option<&str>,
    reply: Box<dyn Reply<Data = Response>>,
    cleanup: F,
) -> io::Result<SpawnResult>
where
    F: FnOnce(ProcessId) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let (shell, shell_flag) = match family {
        DockerFamily::Unix => ("sh", "-c"),
        DockerFamily::Windows => ("cmd", "/c"),
    };

    let env_vec: Vec<String> = environment
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();

    let created = client
        .create_exec(
            container,
            CreateExecOptions {
                cmd: Some(vec![
                    shell.to_string(),
                    shell_flag.to_string(),
                    cmd.to_string(),
                ]),
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                env: if env_vec.is_empty() {
                    None
                } else {
                    Some(env_vec)
                },
                working_dir: current_dir
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string()),
                user: user.map(|u| u.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to create process: {}", e)))?;

    let exec_id = created.id.clone();

    let start_result = client
        .start_exec(
            &created.id,
            Some(StartExecOptions {
                detach: false,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to start process: {}", e)))?;

    let id: ProcessId = rand::random();

    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);
    let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
    let (resize_tx, _resize_rx) = mpsc::channel::<PtySize>(4);

    match start_result {
        StartExecResults::Attached {
            mut output,
            mut input,
        } => {
            let stdout_reply = reply.clone_reply();
            let stderr_reply = reply.clone_reply();
            let exit_reply = reply;
            let msg_id = id;
            let exit_client = client.clone();
            let exit_exec_id = exec_id.clone();

            // Reader task: forwards stdout/stderr to the client
            tokio::spawn(async move {
                while let Some(msg) = output.next().await {
                    match msg {
                        Ok(bollard::container::LogOutput::StdOut { message }) => {
                            let _ = stdout_reply.send(Response::ProcStdout {
                                id: msg_id,
                                data: message.to_vec(),
                            });
                        }
                        Ok(bollard::container::LogOutput::StdErr { message }) => {
                            let _ = stderr_reply.send(Response::ProcStderr {
                                id: msg_id,
                                data: message.to_vec(),
                            });
                        }
                        Ok(_) => {}
                        Err(e) => {
                            error!("Error reading output for process {}: {}", msg_id, e);
                            break;
                        }
                    }
                }

                // Retrieve the real exit code from the Docker exec API
                let (success, code) = match exit_client.inspect_exec(&exit_exec_id).await {
                    Ok(inspect) => {
                        let exit_code = inspect.exit_code.unwrap_or(-1);
                        (exit_code == 0, Some(exit_code as i32))
                    }
                    Err(e) => {
                        warn!(
                            "Failed to inspect exec {} for process {}: {}",
                            exit_exec_id, msg_id, e
                        );
                        (false, None)
                    }
                };

                let _ = exit_reply.send(Response::ProcDone {
                    id: msg_id,
                    success,
                    code,
                });

                cleanup(msg_id).await;
            });

            // Writer task: forwards stdin from the client and handles kill
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;

                loop {
                    tokio::select! {
                        data = stdin_rx.recv() => {
                            match data {
                                Some(data) => {
                                    if let Err(e) = input.write_all(&data).await {
                                        debug!("Failed to write stdin for process {}: {}", id, e);
                                        break;
                                    }
                                }
                                None => break,
                            }
                        }
                        _ = kill_rx.recv() => {
                            debug!("Kill signal received for process {}", id);
                            let _ = input.shutdown().await;
                            break;
                        }
                    }
                }
            });
        }
        StartExecResults::Detached => {
            return Err(io::Error::other("Started in detached mode unexpectedly"));
        }
    }

    Ok(SpawnResult {
        id,
        stdin: stdin_tx,
        killer: kill_tx,
        resizer: resize_tx,
        exec_id,
    })
}

/// Spawns a PTY process in a Docker container.
#[allow(clippy::too_many_arguments)]
pub async fn spawn_pty<F, Fut>(
    client: &Docker,
    container: &str,
    cmd: &str,
    environment: Environment,
    current_dir: Option<std::path::PathBuf>,
    size: PtySize,
    family: DockerFamily,
    user: Option<&str>,
    reply: Box<dyn Reply<Data = Response>>,
    cleanup: F,
) -> io::Result<SpawnResult>
where
    F: FnOnce(ProcessId) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let (shell, shell_flag) = match family {
        DockerFamily::Unix => ("sh", "-c"),
        DockerFamily::Windows => ("cmd", "/c"),
    };

    let env_vec: Vec<String> = environment
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect();

    // For PTY mode, if cmd is empty, just open a shell
    let cmd_parts = if cmd.is_empty() {
        vec![shell.to_string()]
    } else {
        vec![shell.to_string(), shell_flag.to_string(), cmd.to_string()]
    };

    let created = client
        .create_exec(
            container,
            CreateExecOptions {
                cmd: Some(cmd_parts),
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                tty: Some(true),
                env: if env_vec.is_empty() {
                    None
                } else {
                    Some(env_vec)
                },
                working_dir: current_dir
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string()),
                user: user.map(|u| u.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to create PTY process: {}", e)))?;

    let exec_id = created.id.clone();

    let start_result = client
        .start_exec(
            &created.id,
            Some(StartExecOptions {
                detach: false,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to start PTY process: {}", e)))?;

    // Resize the PTY after starting — the Docker API requires the exec to be
    // running before resize_exec is valid. This is best-effort; if it fails,
    // the writer task's resize handler will correct the size on the next
    // client-initiated resize.
    let _ = client
        .resize_exec(
            &exec_id,
            ResizeExecOptions {
                height: size.rows,
                width: size.cols,
            },
        )
        .await;

    let id: ProcessId = rand::random();

    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(32);
    let (kill_tx, mut kill_rx) = mpsc::channel::<()>(1);
    let (resize_tx, mut resize_rx) = mpsc::channel::<PtySize>(4);

    match start_result {
        StartExecResults::Attached {
            mut output,
            mut input,
        } => {
            let stdout_reply = reply.clone_reply();
            let exit_reply = reply;
            let msg_id = id;
            let exit_client = client.clone();
            let exit_exec_id = exec_id.clone();

            // Reader task: PTY combines stdout+stderr into one stream
            tokio::spawn(async move {
                while let Some(msg) = output.next().await {
                    match msg {
                        Ok(bollard::container::LogOutput::StdOut { message })
                        | Ok(bollard::container::LogOutput::Console { message }) => {
                            let _ = stdout_reply.send(Response::ProcStdout {
                                id: msg_id,
                                data: message.to_vec(),
                            });
                        }
                        Ok(_) => {}
                        Err(e) => {
                            error!("Error reading PTY output for process {}: {}", msg_id, e);
                            break;
                        }
                    }
                }

                // Retrieve the real exit code from the Docker exec API
                let (success, code) = match exit_client.inspect_exec(&exit_exec_id).await {
                    Ok(inspect) => {
                        let exit_code = inspect.exit_code.unwrap_or(-1);
                        (exit_code == 0, Some(exit_code as i32))
                    }
                    Err(e) => {
                        warn!(
                            "Failed to inspect exec {} for PTY process {}: {}",
                            exit_exec_id, msg_id, e
                        );
                        (false, None)
                    }
                };

                let _ = exit_reply.send(Response::ProcDone {
                    id: msg_id,
                    success,
                    code,
                });

                cleanup(msg_id).await;
            });

            // Writer task with resize support
            let resize_client = client.clone();
            let resize_exec_id = exec_id.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;

                loop {
                    tokio::select! {
                        data = stdin_rx.recv() => {
                            match data {
                                Some(data) => {
                                    if let Err(e) = input.write_all(&data).await {
                                        debug!("Failed to write PTY stdin: {}", e);
                                        break;
                                    }
                                }
                                None => break,
                            }
                        }
                        _ = kill_rx.recv() => {
                            debug!("Kill signal received for PTY process {}", id);
                            let _ = input.shutdown().await;
                            break;
                        }
                        new_size = resize_rx.recv() => {
                            if let Some(new_size) = new_size {
                                let _ = resize_client.resize_exec(
                                    &resize_exec_id,
                                    ResizeExecOptions {
                                        height: new_size.rows,
                                        width: new_size.cols,
                                    },
                                ).await;
                            }
                        }
                    }
                }
            });
        }
        StartExecResults::Detached => {
            return Err(io::Error::other(
                "PTY started in detached mode unexpectedly",
            ));
        }
    }

    Ok(SpawnResult {
        id,
        stdin: stdin_tx,
        killer: kill_tx,
        resizer: resize_tx,
        exec_id,
    })
}
