use crate::core::{
    constants::MAX_PIPE_CHUNK_SIZE,
    data::{
        self, DirEntry, FileType, Request, RequestData, Response, ResponseData, RunningProcess,
    },
    state::{Process, ServerState},
};
use futures::future;
use log::*;
use std::{
    env,
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
        tenant: Arc<String>,
        addr: SocketAddr,
        state: HState,
        data: RequestData,
        tx: Reply,
    ) -> Result<ResponseData, Box<dyn std::error::Error>> {
        match data {
            RequestData::FileRead { path } => file_read(path).await,
            RequestData::FileReadText { path } => file_read_text(path).await,
            RequestData::FileWrite { path, data } => file_write(path, data).await,
            RequestData::FileWriteText { path, text } => file_write(path, text).await,
            RequestData::FileAppend { path, data } => file_append(path, data).await,
            RequestData::FileAppendText { path, text } => file_append(path, text).await,
            RequestData::DirRead {
                path,
                depth,
                absolute,
                canonicalize,
                include_root,
            } => dir_read(path, depth, absolute, canonicalize, include_root).await,
            RequestData::DirCreate { path, all } => dir_create(path, all).await,
            RequestData::Remove { path, force } => remove(path, force).await,
            RequestData::Copy { src, dst } => copy(src, dst).await,
            RequestData::Rename { src, dst } => rename(src, dst).await,
            RequestData::Metadata { path, canonicalize } => metadata(path, canonicalize).await,
            RequestData::ProcRun { cmd, args } => {
                proc_run(tenant.to_string(), addr, state, tx, cmd, args).await
            }
            RequestData::ProcKill { id } => proc_kill(state, id).await,
            RequestData::ProcStdin { id, data } => proc_stdin(state, id, data).await,
            RequestData::ProcList {} => proc_list(state).await,
            RequestData::SystemInfo {} => system_info().await,
        }
    }

    let tenant = Arc::new(req.tenant.clone());

    // Build up a collection of tasks to run independently
    let mut payload_tasks = Vec::new();
    for data in req.payload {
        let tenant_2 = Arc::clone(&tenant);
        let state_2 = Arc::clone(&state);
        let tx_2 = tx.clone();
        payload_tasks.push(tokio::spawn(async move {
            match inner(tenant_2, addr, state_2, data, tx_2).await {
                Ok(data) => data,
                Err(x) => ResponseData::Error {
                    description: x.to_string(),
                },
            }
        }));
    }

    // Collect the results of our tasks into the payload entries
    let payload = future::join_all(payload_tasks)
        .await
        .into_iter()
        .map(|x| match x {
            Ok(x) => x,
            Err(x) => ResponseData::Error {
                description: x.to_string(),
            },
        })
        .collect();

    let res = Response::new(req.tenant, Some(req.id), payload);

    debug!(
        "<Client @ {}> Sending response of type{} {}",
        addr,
        if res.payload.len() > 1 { "s" } else { "" },
        res.to_payload_type_string()
    );

    // Send out our primary response from processing the request
    tx.send(res).await
}

async fn file_read(path: PathBuf) -> Result<ResponseData, Box<dyn Error>> {
    Ok(ResponseData::Blob {
        data: tokio::fs::read(path).await?,
    })
}

async fn file_read_text(path: PathBuf) -> Result<ResponseData, Box<dyn Error>> {
    Ok(ResponseData::Text {
        data: tokio::fs::read_to_string(path).await?,
    })
}

async fn file_write(path: PathBuf, data: impl AsRef<[u8]>) -> Result<ResponseData, Box<dyn Error>> {
    tokio::fs::write(path, data).await?;
    Ok(ResponseData::Ok)
}

async fn file_append(
    path: PathBuf,
    data: impl AsRef<[u8]>,
) -> Result<ResponseData, Box<dyn Error>> {
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await?;
    file.write_all(data.as_ref()).await?;
    Ok(ResponseData::Ok)
}

async fn dir_read(
    path: PathBuf,
    depth: usize,
    absolute: bool,
    canonicalize: bool,
    include_root: bool,
) -> Result<ResponseData, Box<dyn Error>> {
    // Canonicalize our provided path to ensure that it is exists, not a loop, and absolute
    let root_path = tokio::fs::canonicalize(path).await?;

    // Traverse, but don't include root directory in entries (hence min depth 1), unless indicated
    // to do so (min depth 0)
    let dir = WalkDir::new(root_path.as_path())
        .min_depth(if include_root { 0 } else { 1 })
        .sort_by_file_name();

    // If depth > 0, will recursively traverse to specified max depth, otherwise
    // performs infinite traversal
    let dir = if depth > 0 { dir.max_depth(depth) } else { dir };

    // Determine our entries and errors
    let mut entries = Vec::new();
    let mut errors = Vec::new();

    #[inline]
    fn map_file_type(ft: std::fs::FileType) -> FileType {
        if ft.is_dir() {
            FileType::Dir
        } else if ft.is_file() {
            FileType::File
        } else {
            FileType::SymLink
        }
    }

    for entry in dir {
        match entry.map_err(data::Error::from) {
            // For entries within the root, we want to transform the path based on flags
            Ok(e) if e.depth() > 0 => {
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
                    file_type: map_file_type(e.file_type()),
                    depth: e.depth(),
                });
            }

            // For the root, we just want to echo back the entry as is
            Ok(e) => {
                entries.push(DirEntry {
                    path: e.path().to_path_buf(),
                    file_type: map_file_type(e.file_type()),
                    depth: e.depth(),
                });
            }

            Err(x) => errors.push(x),
        }
    }

    Ok(ResponseData::DirEntries { entries, errors })
}

async fn dir_create(path: PathBuf, all: bool) -> Result<ResponseData, Box<dyn Error>> {
    if all {
        tokio::fs::create_dir_all(path).await?;
    } else {
        tokio::fs::create_dir(path).await?;
    }

    Ok(ResponseData::Ok)
}

async fn remove(path: PathBuf, force: bool) -> Result<ResponseData, Box<dyn Error>> {
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

    Ok(ResponseData::Ok)
}

async fn copy(src: PathBuf, dst: PathBuf) -> Result<ResponseData, Box<dyn Error>> {
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

    Ok(ResponseData::Ok)
}

async fn rename(src: PathBuf, dst: PathBuf) -> Result<ResponseData, Box<dyn Error>> {
    tokio::fs::rename(src, dst).await?;

    Ok(ResponseData::Ok)
}

async fn metadata(path: PathBuf, canonicalize: bool) -> Result<ResponseData, Box<dyn Error>> {
    let metadata = tokio::fs::metadata(path.as_path()).await?;
    let canonicalized_path = if canonicalize {
        Some(tokio::fs::canonicalize(path).await?)
    } else {
        None
    };

    Ok(ResponseData::Metadata {
        canonicalized_path,
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
    })
}

async fn proc_run(
    tenant: String,
    addr: SocketAddr,
    state: HState,
    tx: Reply,
    cmd: String,
    args: Vec<String>,
) -> Result<ResponseData, Box<dyn Error>> {
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
                            vec![ResponseData::ProcStdout { id, data }],
                        );
                        debug!(
                            "<Client @ {}> Sending response of type{} {}",
                            addr,
                            if res.payload.len() > 1 { "s" } else { "" },
                            res.to_payload_type_string()
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
                            vec![ResponseData::ProcStderr { id, data }],
                        );
                        debug!(
                            "<Client @ {}> Sending response of type{} {}",
                            addr,
                            if res.payload.len() > 1 { "s" } else { "" },
                            res.to_payload_type_string()
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
                            vec![ResponseData::ProcDone { id, success, code }]
                        );
                        debug!(
                            "<Client @ {}> Sending response of type{} {}",
                            addr,
                            if res.payload.len() > 1 { "s" } else { "" },
                            res.to_payload_type_string()
                        );
                        if let Err(_) = tx.send(res).await {
                            error!("Failed to send done for process {}!", id);
                        }
                    }
                    Err(x) => {
                        let res = Response::new(tenant.as_str(), None, vec![ResponseData::Error {
                            description: x.to_string()
                        }]);
                        debug!(
                            "<Client @ {}> Sending response of type{} {}",
                            addr,
                            if res.payload.len() > 1 { "s" } else { "" },
                            res.to_payload_type_string()
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


                let res = Response::new(tenant.as_str(), None, vec![ResponseData::ProcDone {
                    id, success: false, code: None
                }]);
                debug!(
                    "<Client @ {}> Sending response of type{} {}",
                    addr,
                    if res.payload.len() > 1 { "s" } else { "" },
                    res.to_payload_type_string()
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

    Ok(ResponseData::ProcStart { id })
}

async fn proc_kill(state: HState, id: usize) -> Result<ResponseData, Box<dyn Error>> {
    if let Some(process) = state.lock().await.processes.remove(&id) {
        process.kill_tx.send(()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Unable to send kill signal to process",
            )
        })?;
    }

    Ok(ResponseData::Ok)
}

async fn proc_stdin(
    state: HState,
    id: usize,
    data: String,
) -> Result<ResponseData, Box<dyn Error>> {
    if let Some(process) = state.lock().await.processes.get(&id) {
        process.stdin_tx.send(data).await.map_err(|_| {
            io::Error::new(io::ErrorKind::BrokenPipe, "Unable to send stdin to process")
        })?;
    }

    Ok(ResponseData::Ok)
}

async fn proc_list(state: HState) -> Result<ResponseData, Box<dyn Error>> {
    Ok(ResponseData::ProcEntries {
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

async fn system_info() -> Result<ResponseData, Box<dyn Error>> {
    Ok(ResponseData::SystemInfo {
        family: env::consts::FAMILY.to_string(),
        os: env::consts::OS.to_string(),
        arch: env::consts::ARCH.to_string(),
        current_dir: env::current_dir().unwrap_or_default(),
        main_separator: std::path::MAIN_SEPARATOR,
    })
}
