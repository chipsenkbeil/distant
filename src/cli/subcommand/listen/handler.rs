use crate::core::{
    constants::MAX_PIPE_CHUNK_SIZE,
    data::{
        self, DirEntry, FileType, Metadata, Request, RequestPayload, Response, ResponsePayload,
        RunningProcess,
    },
    state::{Process, ServerState},
};
use log::*;
use std::{
    error::Error,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::SystemTime,
};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, oneshot, Mutex},
};
use walkdir::WalkDir;

pub type Reply = mpsc::Sender<Response>;
type HState = Arc<Mutex<ServerState<SocketAddr>>>;

/// Processes the provided request, sending replies using the given sender
pub(super) async fn process(
    addr: SocketAddr,
    state: HState,
    req: Request,
    tx: Reply,
) -> Result<(), mpsc::error::SendError<Response>> {
    async fn inner(
        tenant: String,
        addr: SocketAddr,
        state: HState,
        payload: RequestPayload,
        tx: Reply,
    ) -> Result<ResponsePayload, Box<dyn std::error::Error>> {
        match payload {
            RequestPayload::FileRead { path } => file_read(path).await,
            RequestPayload::FileReadText { path } => file_read_text(path).await,
            RequestPayload::FileWrite { path, data } => file_write(path, data).await,
            RequestPayload::FileWriteText { path, text } => file_write(path, text).await,
            RequestPayload::FileAppend { path, data } => file_append(path, data).await,
            RequestPayload::FileAppendText { path, text } => file_append(path, text).await,
            RequestPayload::DirRead {
                path,
                depth,
                absolute,
                canonicalize,
            } => dir_read(path, depth, absolute, canonicalize).await,
            RequestPayload::DirCreate { path, all } => dir_create(path, all).await,
            RequestPayload::Remove { path, force } => remove(path, force).await,
            RequestPayload::Copy { src, dst } => copy(src, dst).await,
            RequestPayload::Rename { src, dst } => rename(src, dst).await,
            RequestPayload::Metadata { path } => metadata(path).await,
            RequestPayload::ProcRun { cmd, args } => {
                proc_run(tenant.to_string(), addr, state, tx, cmd, args).await
            }
            RequestPayload::ProcKill { id } => proc_kill(state, id).await,
            RequestPayload::ProcStdin { id, data } => proc_stdin(state, id, data).await,
            RequestPayload::ProcList {} => proc_list(state).await,
        }
    }

    let tenant = req.tenant.clone();
    let res = Response::new(
        req.tenant,
        Some(req.id),
        match inner(tenant, addr, state, req.payload, tx.clone()).await {
            Ok(payload) => payload,
            Err(x) => ResponsePayload::Error {
                description: x.to_string(),
            },
        },
    );

    debug!(
        "<Client @ {}> Sending response of type {}",
        addr,
        res.payload.as_ref()
    );

    // Send out our primary response from processing the request
    tx.send(res).await
}

async fn file_read(path: PathBuf) -> Result<ResponsePayload, Box<dyn Error>> {
    Ok(ResponsePayload::Blob {
        data: tokio::fs::read(path).await?,
    })
}

async fn file_read_text(path: PathBuf) -> Result<ResponsePayload, Box<dyn Error>> {
    Ok(ResponsePayload::Text {
        data: tokio::fs::read_to_string(path).await?,
    })
}

async fn file_write(
    path: PathBuf,
    data: impl AsRef<[u8]>,
) -> Result<ResponsePayload, Box<dyn Error>> {
    tokio::fs::write(path, data).await?;
    Ok(ResponsePayload::Ok)
}

async fn file_append(
    path: PathBuf,
    data: impl AsRef<[u8]>,
) -> Result<ResponsePayload, Box<dyn Error>> {
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await?;
    file.write_all(data.as_ref()).await?;
    Ok(ResponsePayload::Ok)
}

async fn dir_read(
    path: PathBuf,
    depth: usize,
    absolute: bool,
    canonicalize: bool,
) -> Result<ResponsePayload, Box<dyn Error>> {
    // Canonicalize our provided path to ensure that it is exists, not a loop, and absolute
    let root_path = tokio::fs::canonicalize(path).await?;

    // Traverse, but don't include root directory in entries (hence min depth 1)
    let dir = WalkDir::new(root_path.as_path()).min_depth(1);

    // If depth > 0, will recursively traverse to specified max depth, otherwise
    // performs infinite traversal
    let dir = if depth > 0 { dir.max_depth(depth) } else { dir };

    // Determine our entries and errors
    let mut entries = Vec::new();
    let mut errors = Vec::new();

    for entry in dir {
        match entry.map_err(data::Error::from) {
            Ok(e) => {
                // Canonicalize the path if specified, otherwise just return
                // the path as is
                let mut path = if canonicalize {
                    match tokio::fs::canonicalize(e.path()).await {
                        Ok(path) => path,
                        Err(x) => {
                            errors.push(data::Error::from(x));
                            continue;
                        }
                    }
                } else {
                    e.path().to_path_buf()
                };

                // Strip the path of its prefix based if not flagged as absolute
                if !absolute {
                    // NOTE: In the situation where we canonicalized the path earlier,
                    //       there is no guarantee that our root path is still the
                    //       parent of the symlink's destination; so, in that case we MUST just
                    //       return the path if the strip_prefix fails
                    path = path
                        .strip_prefix(root_path.as_path())
                        .map(Path::to_path_buf)
                        .unwrap_or(path);
                };

                entries.push(DirEntry {
                    path,
                    file_type: if e.file_type().is_dir() {
                        FileType::Dir
                    } else if e.file_type().is_file() {
                        FileType::File
                    } else {
                        FileType::SymLink
                    },
                    depth: e.depth(),
                });
            }
            Err(x) => errors.push(x),
        }
    }

    Ok(ResponsePayload::DirEntries { entries, errors })
}

async fn dir_create(path: PathBuf, all: bool) -> Result<ResponsePayload, Box<dyn Error>> {
    if all {
        tokio::fs::create_dir_all(path).await?;
    } else {
        tokio::fs::create_dir(path).await?;
    }

    Ok(ResponsePayload::Ok)
}

async fn remove(path: PathBuf, force: bool) -> Result<ResponsePayload, Box<dyn Error>> {
    let path_metadata = tokio::fs::metadata(path.as_path()).await?;
    if path_metadata.is_dir() {
        if force {
            tokio::fs::remove_dir_all(path).await?;
        } else {
            tokio::fs::remove_dir(path).await?;
        }
    } else {
        tokio::fs::remove_file(path).await?;
    }

    Ok(ResponsePayload::Ok)
}

async fn copy(src: PathBuf, dst: PathBuf) -> Result<ResponsePayload, Box<dyn Error>> {
    let src_metadata = tokio::fs::metadata(src.as_path()).await?;
    if src_metadata.is_dir() {
        for entry in WalkDir::new(src.as_path())
            .min_depth(1)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.file_type().is_file() || e.path_is_symlink())
        {
            let entry = entry?;

            // Get unique portion of path relative to src
            // NOTE: Because we are traversing files that are all within src, this
            //       should always succeed
            let local_src = entry.path().strip_prefix(src.as_path()).unwrap();

            // Get the file without any directories
            let local_src_file_name = local_src.file_name().unwrap();

            // Get the directory housing the file
            // NOTE: Because we enforce files/symlinks, there will always be a parent
            let local_src_dir = local_src.parent().unwrap();

            // Map out the path to the destination
            let dst_parent_dir = dst.join(local_src_dir);

            // Create the destination directory for the file when copying
            tokio::fs::create_dir_all(dst_parent_dir.as_path()).await?;

            // Perform copying from entry to destination
            let dst_file = dst_parent_dir.join(local_src_file_name);
            tokio::fs::copy(entry.path(), dst_file).await?;
        }
    } else {
        tokio::fs::copy(src, dst).await?;
    }

    Ok(ResponsePayload::Ok)
}

async fn rename(src: PathBuf, dst: PathBuf) -> Result<ResponsePayload, Box<dyn Error>> {
    tokio::fs::rename(src, dst).await?;

    Ok(ResponsePayload::Ok)
}

async fn metadata(path: PathBuf) -> Result<ResponsePayload, Box<dyn Error>> {
    let metadata = tokio::fs::metadata(path).await?;

    Ok(ResponsePayload::Metadata {
        data: Metadata {
            accessed: metadata
                .accessed()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_millis()),
            created: metadata
                .created()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_millis()),
            modified: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_millis()),
            len: metadata.len(),
            readonly: metadata.permissions().readonly(),
            file_type: if metadata.is_dir() {
                FileType::Dir
            } else if metadata.is_file() {
                FileType::File
            } else {
                FileType::SymLink
            },
        },
    })
}

async fn proc_run(
    tenant: String,
    addr: SocketAddr,
    state: HState,
    tx: Reply,
    cmd: String,
    args: Vec<String>,
) -> Result<ResponsePayload, Box<dyn Error>> {
    let id = rand::random();

    let mut child = Command::new(cmd.to_string())
        .args(args.clone())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Spawn a task that sends stdout as a response
    let tx_2 = tx.clone();
    let tenant_2 = tenant.clone();
    let mut stdout = child.stdout.take().unwrap();
    let stdout_task = tokio::spawn(async move {
        let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
        loop {
            match stdout.read(&mut buf).await {
                Ok(n) if n > 0 => match String::from_utf8(buf[..n].to_vec()) {
                    Ok(data) => {
                        let res = Response::new(
                            tenant_2.as_str(),
                            None,
                            ResponsePayload::ProcStdout { id, data },
                        );
                        debug!(
                            "<Client @ {}> Sending response of type {}",
                            addr,
                            res.payload.as_ref()
                        );
                        if let Err(_) = tx_2.send(res).await {
                            break;
                        }
                    }
                    Err(x) => {
                        error!("Invalid data read from stdout pipe: {}", x);
                        break;
                    }
                },
                Ok(_) => break,
                Err(_) => break,
            }
        }
    });

    // Spawn a task that sends stderr as a response
    let tx_2 = tx.clone();
    let tenant_2 = tenant.clone();
    let mut stderr = child.stderr.take().unwrap();
    let stderr_task = tokio::spawn(async move {
        let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
        loop {
            match stderr.read(&mut buf).await {
                Ok(n) if n > 0 => match String::from_utf8(buf[..n].to_vec()) {
                    Ok(data) => {
                        let res = Response::new(
                            tenant_2.as_str(),
                            None,
                            ResponsePayload::ProcStderr { id, data },
                        );
                        debug!(
                            "<Client @ {}> Sending response of type {}",
                            addr,
                            res.payload.as_ref()
                        );
                        if let Err(_) = tx_2.send(res).await {
                            break;
                        }
                    }
                    Err(x) => {
                        error!("Invalid data read from stdout pipe: {}", x);
                        break;
                    }
                },
                Ok(_) => break,
                Err(_) => break,
            }
        }
    });

    // Spawn a task that sends stdin to the process
    let mut stdin = child.stdin.take().unwrap();
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(1);
    tokio::spawn(async move {
        while let Some(line) = stdin_rx.recv().await {
            if let Err(x) = stdin.write_all(line.as_bytes()).await {
                error!("Failed to send stdin to process {}: {}", id, x);
                break;
            }
        }
    });

    // Spawn a task that waits on the process to exit but can also
    // kill the process when triggered
    let (kill_tx, kill_rx) = oneshot::channel();
    tokio::spawn(async move {
        tokio::select! {
            status = child.wait() => {
                if let Err(x) = stderr_task.await {
                    error!("Join on stderr task failed: {}", x);
                }

                if let Err(x) = stdout_task.await {
                    error!("Join on stdout task failed: {}", x);
                }

                match status {
                    Ok(status) => {
                        let success = status.success();
                        let code = status.code();
                        let res = Response::new(
                            tenant.as_str(),
                            None,
                            ResponsePayload::ProcDone { id, success, code }
                        );
                        debug!(
                            "<Client @ {}> Sending response of type {}",
                            addr,
                            res.payload.as_ref()
                        );
                        if let Err(_) = tx.send(res).await {
                            error!("Failed to send done for process {}!", id);
                        }
                    }
                    Err(x) => {
                        let res = Response::new(tenant.as_str(), None, ResponsePayload::Error {
                            description: x.to_string()
                        });
                        debug!(
                            "<Client @ {}> Sending response of type {}",
                            addr,
                            res.payload.as_ref()
                        );
                        if let Err(_) = tx.send(res).await {
                            error!("Failed to send error for waiting on process {}!", id);
                        }
                    }
                }

            },
            _ = kill_rx => {
                if let Err(x) = child.kill().await {
                    error!("Unable to kill process {}: {}", id, x);
                }

                if let Err(x) = stderr_task.await {
                    error!("Join on stderr task failed: {}", x);
                }

                if let Err(x) = stdout_task.await {
                    error!("Join on stdout task failed: {}", x);
                }


                let res = Response::new(tenant.as_str(), None, ResponsePayload::ProcDone {
                    id, success: false, code: None
                });
                debug!(
                    "<Client @ {}> Sending response of type {}",
                    addr,
                    res.payload.as_ref()
                );
                if let Err(_) = tx
                    .send(res)
                    .await
                {
                    error!("Failed to send done for process {}!", id);
                }
            }
        }
    });

    // Update our state with the new process
    let process = Process {
        cmd,
        args,
        id,
        stdin_tx,
        kill_tx,
    };
    state.lock().await.push_process(addr, process);

    Ok(ResponsePayload::ProcStart { id })
}

async fn proc_kill(state: HState, id: usize) -> Result<ResponsePayload, Box<dyn Error>> {
    if let Some(process) = state.lock().await.processes.remove(&id) {
        process.kill_tx.send(()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Unable to send kill signal to process",
            )
        })?;
    }

    Ok(ResponsePayload::Ok)
}

async fn proc_stdin(
    state: HState,
    id: usize,
    data: String,
) -> Result<ResponsePayload, Box<dyn Error>> {
    if let Some(process) = state.lock().await.processes.get(&id) {
        process.stdin_tx.send(data).await.map_err(|_| {
            io::Error::new(io::ErrorKind::BrokenPipe, "Unable to send stdin to process")
        })?;
    }

    Ok(ResponsePayload::Ok)
}

async fn proc_list(state: HState) -> Result<ResponsePayload, Box<dyn Error>> {
    Ok(ResponsePayload::ProcEntries {
        entries: state
            .lock()
            .await
            .processes
            .values()
            .map(|p| RunningProcess {
                cmd: p.cmd.to_string(),
                args: p.args.clone(),
                id: p.id,
            })
            .collect(),
    })
}
