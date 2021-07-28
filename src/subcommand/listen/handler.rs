use super::{Process, State};
use crate::data::{
    DirEntry, FileType, Request, RequestPayload, Response, ResponsePayload, RunningProcess,
};
use log::*;
use std::{error::Error, path::PathBuf, process::Stdio, sync::Arc};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, oneshot, Mutex},
};
use walkdir::WalkDir;

pub type Reply = mpsc::Sender<Response>;
type HState = Arc<Mutex<State>>;

/// Processes the provided request, sending replies using the given sender
pub(super) async fn process(
    client_id: usize,
    state: HState,
    req: Request,
    tx: Reply,
) -> Result<(), mpsc::error::SendError<Response>> {
    async fn inner(
        client_id: usize,
        state: HState,
        payload: RequestPayload,
        tx: Reply,
    ) -> Result<ResponsePayload, Box<dyn std::error::Error>> {
        match payload {
            RequestPayload::FileRead { path } => file_read(path).await,
            RequestPayload::FileReadText { path } => file_read_text(path).await,
            RequestPayload::FileWrite { path, data, .. } => file_write(path, data).await,
            RequestPayload::FileAppend { path, data, .. } => file_append(path, data).await,
            RequestPayload::DirRead { path, all } => dir_read(path, all).await,
            RequestPayload::DirCreate { path, all } => dir_create(path, all).await,
            RequestPayload::Remove { path, force } => remove(path, force).await,
            RequestPayload::Copy { src, dst } => copy(src, dst).await,
            RequestPayload::Rename { src, dst } => rename(src, dst).await,
            RequestPayload::ProcRun { cmd, args, detach } => {
                proc_run(client_id, state, tx, cmd, args, detach).await
            }
            RequestPayload::ProcConnect { id } => proc_connect(id).await,
            RequestPayload::ProcKill { id } => proc_kill(state, id).await,
            RequestPayload::ProcStdin { id, data } => proc_stdin(state, id, data).await,
            RequestPayload::ProcList {} => proc_list(state).await,
        }
    }

    let res = Response::from_payload_with_origin(
        match inner(client_id, state, req.payload, tx.clone()).await {
            Ok(payload) => payload,
            Err(x) => ResponsePayload::Error {
                description: x.to_string(),
            },
        },
        req.id,
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

async fn file_write(path: PathBuf, data: Vec<u8>) -> Result<ResponsePayload, Box<dyn Error>> {
    tokio::fs::write(path, data).await?;
    Ok(ResponsePayload::Ok)
}

async fn file_append(path: PathBuf, data: Vec<u8>) -> Result<ResponsePayload, Box<dyn Error>> {
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .await?;
    file.write_all(&data).await?;
    Ok(ResponsePayload::Ok)
}

async fn dir_read(path: PathBuf, all: bool) -> Result<ResponsePayload, Box<dyn Error>> {
    // Traverse, but don't include root directory in entries (hence min depth 1)
    let dir = WalkDir::new(path.as_path()).min_depth(1);

    // If all, will recursively traverse, otherwise just return directly from dir
    let dir = if all { dir } else { dir.max_depth(1) };

    // TODO: Support both returning errors and successfully-traversed entries
    // TODO: Support returning full paths instead of always relative?
    Ok(ResponsePayload::DirEntries {
        entries: dir
            .into_iter()
            .map(|e| {
                e.map(|e| DirEntry {
                    path: e.path().strip_prefix(path.as_path()).unwrap().to_path_buf(),
                    file_type: if e.file_type().is_dir() {
                        FileType::Dir
                    } else if e.file_type().is_file() {
                        FileType::File
                    } else {
                        FileType::SymLink
                    },
                    depth: e.depth(),
                })
            })
            .collect::<Result<Vec<DirEntry>, walkdir::Error>>()?,
    })
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

async fn proc_run(
    client_id: usize,
    state: HState,
    tx: Reply,
    cmd: String,
    args: Vec<String>,
    detach: bool,
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
    let mut stdout = child.stdout.take().unwrap();
    tokio::spawn(async move {
        loop {
            let mut data = Vec::new();
            match stdout.read_to_end(&mut data).await {
                Ok(_) => {
                    if let Err(_) = tx_2
                        .send(Response::from(ResponsePayload::ProcStdout { id, data }))
                        .await
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Spawn a task that sends stderr as a response
    let mut stderr = child.stderr.take().unwrap();
    tokio::spawn(async move {
        loop {
            let mut data = Vec::new();
            match stderr.read_to_end(&mut data).await {
                Ok(_) => {
                    if let Err(_) = tx
                        .send(Response::from(ResponsePayload::ProcStderr { id, data }))
                        .await
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Spawn a task that sends stdin to the process
    // TODO: Should this be configurable?
    let mut stdin = child.stdin.take().unwrap();
    let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(1);
    tokio::spawn(async move {
        while let Some(data) = stdin_rx.recv().await {
            if let Err(x) = stdin.write_all(&data).await {
                error!("Failed to send stdin to process {}: {}", id, x);
                break;
            }
        }
    });

    // Spawn a task that kills the process when triggered
    let (kill_tx, kill_rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = kill_rx.await;
        if let Err(x) = child.kill().await {
            error!("Unable to kill process {}: {}", id, x);
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
    state.lock().await.processes.insert(id, process);

    // If we are not detaching from process, we want to associate it with our client
    if !detach {
        state
            .lock()
            .await
            .client_processes
            .entry(client_id)
            .or_insert(Vec::new())
            .push(id);
    }

    Ok(ResponsePayload::ProcStart { id })
}

async fn proc_connect(id: usize) -> Result<ResponsePayload, Box<dyn Error>> {
    todo!();
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
    data: Vec<u8>,
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
