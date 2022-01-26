use async_compat::CompatExt;
use distant_core::{
    data::{
        DirEntry, Error as DistantError, FileType, Metadata, PtySize, RunningProcess, SystemInfo,
    },
    Request, RequestData, Response, ResponseData,
};
use futures::future;
use log::*;
use std::{
    collections::HashMap,
    future::Future,
    io::{self, Read, Write},
    path::{Component, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio::sync::{mpsc, Mutex};
use wezterm_ssh::{
    Child, ExecResult, FilePermissions, OpenFileType, OpenOptions, Session, WriteMode,
};

/// Stdin writer for a process
pub struct ProcessStdin(mpsc::Sender<Vec<u8>>);

/// Killer for a process, capable of notifying event loop to kill process
pub struct ProcessKiller(mpsc::Sender<()>);

/// Result of spawning a process, containing means to send stdin, means to kill the process,
/// and the initialization function to use to start processing stdin, stdout, and stderr
pub struct SpawnResult {
    pub stdin: ProcessStdin,
    pub killer: ProcessKiller,
    pub initialize: Box<dyn FnOnce() + Send>,
}

/// Spawns a non-pty process, returning a function that initializes processing
/// stdin, stdout, and stderr once called (for lazy processing)
pub async fn spawn_simple(session: &Session, cmd: &str) -> io::Result<SpawnResult> {
    let ExecResult {
        mut stdin,
        mut stdout,
        mut stderr,
        mut child,
    } = session
        .exec(&cmd, None)
        .compat()
        .await
        .map_err(to_other_error)?;

    // Force stdin, stdout, and stderr to be nonblocking
    stdin
        .set_non_blocking(true)
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
    stdout
        .set_non_blocking(true)
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
    stderr
        .set_non_blocking(true)
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

    // Check if the process died immediately and report
    // an error if that's the case
    if let Ok(Some(exit_status)) = child.try_wait() {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("Process exited early: {:?}", exit_status),
        ));
    }

    let (stdin_tx, mut stdin_rx) = mpsc::channel(1);
    let (kill_tx, mut kill_rx) = mpsc::channel(1);

    let stdin = ProcessStdin(stdin_tx);
    let killer = ProcessKiller(kill_tx);

    let post_hook = Box::new(move || {
        // Spawn a task that sends stdout as a response
        let mut reply_2 = reply.clone();
        let stdout_task = tokio::spawn(async move {
            let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
            loop {
                match stdout.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let payload = vec![ResponseData::ProcStdout {
                            id,
                            data: buf[..n].to_vec(),
                        }];
                        if !reply_2(payload).await {
                            error!("<Ssh | Proc {}> Stdout channel closed", id);
                            break;
                        }

                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
                    }
                    Ok(_) => break,
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
                    }
                    Err(x) => {
                        error!("<Ssh | Proc {}> Stdout unexpectedly closed: {}", id, x);
                        break;
                    }
                }
            }
        });

        // Spawn a task that sends stderr as a response
        let mut reply_2 = reply.clone();
        let stderr_task = tokio::spawn(async move {
            let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
            loop {
                match stderr.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let payload = vec![ResponseData::ProcStderr {
                            id,
                            data: buf[..n].to_vec(),
                        }];
                        if !reply_2(payload).await {
                            error!("<Ssh | Proc {}> Stderr channel closed", id);
                            break;
                        }

                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
                    }
                    Ok(_) => break,
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
                    }
                    Err(x) => {
                        error!("<Ssh | Proc {}> Stderr unexpectedly closed: {}", id, x);
                        break;
                    }
                }
            }
        });

        let stdin_task = tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                if let Err(x) = stdin.write_all(&line) {
                    error!("<Ssh | Proc {}> Failed to send stdin: {}", id, x);
                    break;
                }
            }
        });

        // Spawn a task that waits on the process to exit but can also
        // kill the process when triggered
        let state_2 = Arc::clone(&state);
        let mut reply_2 = reply.clone();
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

            if let Err(x) = stderr_task.await {
                error!("<Ssh | Proc {}> Join on stderr task failed: {}", id, x);
            }

            if let Err(x) = stdout_task.await {
                error!("<Ssh | Proc {}> Join on stdout task failed: {}", id, x);
            }

            state_2.lock().await.processes.remove(&id);

            let payload = vec![ResponseData::ProcDone {
                id,
                success: !should_kill && success,
                code: if success { Some(0) } else { None },
            }];

            if !reply_2(payload).await {
                error!("<Ssh | Proc {}> Failed to send done", id,);
            }
        });
    });

    debug!(
        "<Ssh | Proc {}> Spawned successfully! Will enter post hook later",
        id
    );
    (id, post_hook)
}

fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::Other, err)
}
