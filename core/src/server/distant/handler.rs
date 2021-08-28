use crate::{
    constants::{MAX_PIPE_CHUNK_SIZE, READ_PAUSE_MILLIS},
    data::{
        self, DirEntry, FileType, Request, RequestData, Response, ResponseData, RunningProcess,
    },
    server::distant::state::{Process, State},
};
use derive_more::{Display, Error, From};
use futures::future;
use log::*;
use std::{
    env,
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
type HState = Arc<Mutex<State>>;

#[derive(Debug, Display, Error, From)]
pub enum ServerError {
    IoError(io::Error),
    WalkDirError(walkdir::Error),
}

impl From<ServerError> for ResponseData {
    fn from(x: ServerError) -> Self {
        match x {
            ServerError::IoError(x) => Self::from(x),
            ServerError::WalkDirError(x) => Self::from(x),
        }
    }
}

/// Processes the provided request, sending replies using the given sender
pub(super) async fn process(
    conn_id: usize,
    state: HState,
    req: Request,
    tx: Reply,
) -> Result<(), mpsc::error::SendError<Response>> {
    async fn inner(
        tenant: Arc<String>,
        conn_id: usize,
        state: HState,
        data: RequestData,
        tx: Reply,
    ) -> Result<ResponseData, ServerError> {
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
            RequestData::Exists { path } => exists(path).await,
            RequestData::Metadata { path, canonicalize } => metadata(path, canonicalize).await,
            RequestData::ProcRun { cmd, args } => {
                proc_run(tenant.to_string(), conn_id, state, tx, cmd, args).await
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
            match inner(tenant_2, conn_id, state_2, data, tx_2).await {
                Ok(data) => data,
                Err(x) => ResponseData::from(x),
            }
        }));
    }

    // Collect the results of our tasks into the payload entries
    let payload = future::join_all(payload_tasks)
        .await
        .into_iter()
        .map(|x| match x {
            Ok(x) => x,
            Err(x) => ResponseData::from(x),
        })
        .collect();

    let res = Response::new(req.tenant, Some(req.id), payload);

    debug!(
        "<Conn @ {}> Sending response of type{} {}",
        conn_id,
        if res.payload.len() > 1 { "s" } else { "" },
        res.to_payload_type_string()
    );

    // Send out our primary response from processing the request
    tx.send(res).await
}

async fn file_read(path: PathBuf) -> Result<ResponseData, ServerError> {
    Ok(ResponseData::Blob {
        data: tokio::fs::read(path).await?,
    })
}

async fn file_read_text(path: PathBuf) -> Result<ResponseData, ServerError> {
    Ok(ResponseData::Text {
        data: tokio::fs::read_to_string(path).await?,
    })
}

async fn file_write(path: PathBuf, data: impl AsRef<[u8]>) -> Result<ResponseData, ServerError> {
    tokio::fs::write(path, data).await?;
    Ok(ResponseData::Ok)
}

async fn file_append(path: PathBuf, data: impl AsRef<[u8]>) -> Result<ResponseData, ServerError> {
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
) -> Result<ResponseData, ServerError> {
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
            FileType::Symlink
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

async fn dir_create(path: PathBuf, all: bool) -> Result<ResponseData, ServerError> {
    if all {
        tokio::fs::create_dir_all(path).await?;
    } else {
        tokio::fs::create_dir(path).await?;
    }

    Ok(ResponseData::Ok)
}

async fn remove(path: PathBuf, force: bool) -> Result<ResponseData, ServerError> {
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

async fn copy(src: PathBuf, dst: PathBuf) -> Result<ResponseData, ServerError> {
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

async fn rename(src: PathBuf, dst: PathBuf) -> Result<ResponseData, ServerError> {
    tokio::fs::rename(src, dst).await?;

    Ok(ResponseData::Ok)
}

async fn exists(path: PathBuf) -> Result<ResponseData, ServerError> {
    // Following experimental `std::fs::try_exists`, which checks the error kind of the
    // metadata lookup to see if it is not found and filters accordingly
    Ok(match tokio::fs::metadata(path.as_path()).await {
        Ok(_) => ResponseData::Exists(true),
        Err(x) if x.kind() == io::ErrorKind::NotFound => ResponseData::Exists(false),
        Err(x) => return Err(ServerError::from(x)),
    })
}

async fn metadata(path: PathBuf, canonicalize: bool) -> Result<ResponseData, ServerError> {
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
            FileType::Symlink
        },
    })
}

async fn proc_run(
    tenant: String,
    conn_id: usize,
    state: HState,
    tx: Reply,
    cmd: String,
    args: Vec<String>,
) -> Result<ResponseData, ServerError> {
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
                            "<Conn @ {}> Sending response of type{} {}",
                            conn_id,
                            if res.payload.len() > 1 { "s" } else { "" },
                            res.to_payload_type_string()
                        );
                        if let Err(_) = tx_2.send(res).await {
                            break;
                        }

                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
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
                            "<Conn @ {}> Sending response of type{} {}",
                            conn_id,
                            if res.payload.len() > 1 { "s" } else { "" },
                            res.to_payload_type_string()
                        );
                        if let Err(_) = tx_2.send(res).await {
                            break;
                        }

                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
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
    let stdin_task = tokio::spawn(async move {
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
    let wait_task = tokio::spawn(async move {
        tokio::select! {
            status = child.wait() => {
                if let Err(x) = stdin_task.await {
                    error!("Join on stdin task failed: {}", x);
                }

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
                            "<Conn @ {}> Sending response of type{} {}",
                            conn_id,
                            if res.payload.len() > 1 { "s" } else { "" },
                            res.to_payload_type_string()
                        );
                        if let Err(_) = tx.send(res).await {
                            error!("Failed to send done for process {}!", id);
                        }
                    }
                    Err(x) => {
                        let res = Response::new(tenant.as_str(), None, vec![ResponseData::from(x)]);
                        debug!(
                            "<Conn @ {}> Sending response of type{} {}",
                            conn_id,
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
                    "<Conn @ {}> Sending response of type{} {}",
                    conn_id,
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
        wait_task,
    };
    state.lock().await.push_process(conn_id, process);

    Ok(ResponseData::ProcStart { id })
}

async fn proc_kill(state: HState, id: usize) -> Result<ResponseData, ServerError> {
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

async fn proc_stdin(state: HState, id: usize, data: String) -> Result<ResponseData, ServerError> {
    if let Some(process) = state.lock().await.processes.get(&id) {
        process.stdin_tx.send(data).await.map_err(|_| {
            io::Error::new(io::ErrorKind::BrokenPipe, "Unable to send stdin to process")
        })?;
    }

    Ok(ResponseData::Ok)
}

async fn proc_list(state: HState) -> Result<ResponseData, ServerError> {
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

async fn system_info() -> Result<ResponseData, ServerError> {
    Ok(ResponseData::SystemInfo {
        family: env::consts::FAMILY.to_string(),
        os: env::consts::OS.to_string(),
        arch: env::consts::ARCH.to_string(),
        current_dir: env::current_dir().unwrap_or_default(),
        main_separator: std::path::MAIN_SEPARATOR,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    fn setup(
        buffer: usize,
    ) -> (
        usize,
        Arc<Mutex<State>>,
        mpsc::Sender<Response>,
        mpsc::Receiver<Response>,
    ) {
        let (tx, rx) = mpsc::channel(buffer);
        (
            rand::random(),
            Arc::new(Mutex::new(State::default())),
            tx,
            rx,
        )
    }

    /// Create a temporary path that does not exist
    fn temppath() -> PathBuf {
        // Deleted when dropped
        NamedTempFile::new().unwrap().into_temp_path().to_path_buf()
    }

    #[tokio::test]
    async fn file_read_should_send_error_if_fails_to_read_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a file and then delete it, keeping just its path
        let path = temppath();

        let req = Request::new("test-tenant", vec![RequestData::FileRead { path }]);

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn file_read_should_send_blob_with_file_contents() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary file and fill it with some contents
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"some file contents").unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileRead {
                path: file.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::Blob { data } => assert_eq!(data, b"some file contents"),
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn file_read_text_should_send_error_if_fails_to_read_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a file and then delete it, keeping just its path
        let path = temppath();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileReadText { path: path }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn file_read_text_should_send_text_with_file_contents() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary file and fill it with some contents
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"some file contents").unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileReadText {
                path: file.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::Text { data } => assert_eq!(data, "some file contents"),
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn file_write_should_send_error_if_fails_to_write_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let path = temppath().join("some_file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileWrite {
                path: path.clone(),
                data: b"some text".to_vec(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we didn't actually create the file
        assert!(!path.exists(), "File created unexpectedly");
    }

    #[tokio::test]
    async fn file_write_should_send_ok_when_successful() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Path should point to a file that does not exist, but all
        // other components leading up to it do
        let path = temppath();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileWrite {
                path: path.clone(),
                data: b"some text".to_vec(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Ok),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we actually did create the file
        // with the associated contents
        assert!(path.exists(), "File not actually created");
        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), "some text");
    }

    #[tokio::test]
    async fn file_write_text_should_send_error_if_fails_to_write_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let path = temppath().join("some_file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileWriteText {
                path: path.clone(),
                text: String::from("some text"),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we didn't actually create the file
        assert!(!path.exists(), "File created unexpectedly");
    }

    #[tokio::test]
    async fn file_write_text_should_send_ok_when_successful() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Path should point to a file that does not exist, but all
        // other components leading up to it do
        let path = temppath();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileWriteText {
                path: path.clone(),
                text: String::from("some text"),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Ok),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we actually did create the file
        // with the associated contents
        assert!(path.exists(), "File not actually created");
        assert_eq!(tokio::fs::read_to_string(path).await.unwrap(), "some text");
    }

    #[tokio::test]
    async fn file_append_should_send_error_if_fails_to_create_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let path = temppath().join("some_file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileAppend {
                path: path.to_path_buf(),
                data: b"some extra contents".to_vec(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we didn't actually create the file
        assert!(!path.exists(), "File created unexpectedly");
    }

    #[tokio::test]
    async fn file_append_should_send_ok_when_successful() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary file and fill it with some contents
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"some file contents").unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileAppend {
                path: file.path().to_path_buf(),
                data: b"some extra contents".to_vec(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Ok),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we actually did append to the file
        assert_eq!(
            tokio::fs::read_to_string(file.path()).await.unwrap(),
            "some file contentssome extra contents"
        );
    }

    #[tokio::test]
    async fn file_append_text_should_send_error_if_fails_to_create_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let path = temppath().join("some_file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileAppendText {
                path: path.to_path_buf(),
                text: String::from("some extra contents"),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we didn't actually create the file
        assert!(!path.exists(), "File created unexpectedly");
    }

    #[tokio::test]
    async fn file_append_text_should_send_ok_when_successful() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary file and fill it with some contents
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"some file contents").unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileAppendText {
                path: file.path().to_path_buf(),
                text: String::from("some extra contents"),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Ok),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we actually did append to the file
        assert_eq!(
            tokio::fs::read_to_string(file.path()).await.unwrap(),
            "some file contentssome extra contents"
        );
    }

    #[tokio::test]
    async fn dir_read_should_send_error_if_directory_does_not_exist() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let path = temppath();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirRead {
                path,
                depth: 0,
                absolute: false,
                canonicalize: false,
                include_root: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    // /root/
    // /root/file1
    // /root/sub1/
    // /root/sub1/file2
    async fn setup_dir() -> TempDir {
        let root_dir = TempDir::new().unwrap();
        let file1 = root_dir.path().join("file1");
        let sub1 = root_dir.path().join("sub1");
        let file2 = sub1.join("file2");

        tokio::fs::write(file1.as_path(), "").await.unwrap();
        tokio::fs::create_dir(sub1.as_path()).await.unwrap();
        tokio::fs::write(file2.as_path(), "").await.unwrap();

        root_dir
    }

    #[tokio::test]
    async fn dir_read_should_support_depth_limits() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirRead {
                path: root_dir.path().to_path_buf(),
                depth: 1,
                absolute: false,
                canonicalize: false,
                include_root: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::DirEntries { entries, .. } => {
                assert_eq!(entries.len(), 2, "Wrong number of entries found");

                assert_eq!(entries[0].file_type, FileType::File);
                assert_eq!(entries[0].path, Path::new("file1"));
                assert_eq!(entries[0].depth, 1);

                assert_eq!(entries[1].file_type, FileType::Dir);
                assert_eq!(entries[1].path, Path::new("sub1"));
                assert_eq!(entries[1].depth, 1);
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn dir_read_should_support_unlimited_depth_using_zero() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirRead {
                path: root_dir.path().to_path_buf(),
                depth: 0,
                absolute: false,
                canonicalize: false,
                include_root: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::DirEntries { entries, .. } => {
                assert_eq!(entries.len(), 3, "Wrong number of entries found");

                assert_eq!(entries[0].file_type, FileType::File);
                assert_eq!(entries[0].path, Path::new("file1"));
                assert_eq!(entries[0].depth, 1);

                assert_eq!(entries[1].file_type, FileType::Dir);
                assert_eq!(entries[1].path, Path::new("sub1"));
                assert_eq!(entries[1].depth, 1);

                assert_eq!(entries[2].file_type, FileType::File);
                assert_eq!(entries[2].path, Path::new("sub1").join("file2"));
                assert_eq!(entries[2].depth, 2);
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn dir_read_should_support_including_directory_in_returned_entries() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirRead {
                path: root_dir.path().to_path_buf(),
                depth: 1,
                absolute: false,
                canonicalize: false,
                include_root: true,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::DirEntries { entries, .. } => {
                assert_eq!(entries.len(), 3, "Wrong number of entries found");

                // NOTE: Root entry is always absolute, resolved path
                assert_eq!(entries[0].file_type, FileType::Dir);
                assert_eq!(entries[0].path, root_dir.path().canonicalize().unwrap());
                assert_eq!(entries[0].depth, 0);

                assert_eq!(entries[1].file_type, FileType::File);
                assert_eq!(entries[1].path, Path::new("file1"));
                assert_eq!(entries[1].depth, 1);

                assert_eq!(entries[2].file_type, FileType::Dir);
                assert_eq!(entries[2].path, Path::new("sub1"));
                assert_eq!(entries[2].depth, 1);
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn dir_read_should_support_returning_absolute_paths() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirRead {
                path: root_dir.path().to_path_buf(),
                depth: 1,
                absolute: true,
                canonicalize: false,
                include_root: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::DirEntries { entries, .. } => {
                assert_eq!(entries.len(), 2, "Wrong number of entries found");
                let root_path = root_dir.path().canonicalize().unwrap();

                assert_eq!(entries[0].file_type, FileType::File);
                assert_eq!(entries[0].path, root_path.join("file1"));
                assert_eq!(entries[0].depth, 1);

                assert_eq!(entries[1].file_type, FileType::Dir);
                assert_eq!(entries[1].path, root_path.join("sub1"));
                assert_eq!(entries[1].depth, 1);
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn dir_read_should_support_returning_canonicalized_paths() {
        todo!("Figure out best way to support symlink tests");
    }

    #[tokio::test]
    async fn dir_create_should_send_error_if_fails() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Make a path that has multiple non-existent components
        // so the creation will fail
        let root_dir = setup_dir().await;
        let path = root_dir.path().join("nested").join("new-dir");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirCreate {
                path: path.to_path_buf(),
                all: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that the directory was not actually created
        assert!(!path.exists(), "Path unexpectedly exists");
    }

    #[tokio::test]
    async fn dir_create_should_send_ok_when_successful() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let root_dir = setup_dir().await;
        let path = root_dir.path().join("new-dir");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirCreate {
                path: path.to_path_buf(),
                all: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Ok),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that the directory was actually created
        assert!(path.exists(), "Directory not created");
    }

    #[tokio::test]
    async fn dir_create_should_support_creating_multiple_dir_components() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let root_dir = setup_dir().await;
        let path = root_dir.path().join("nested").join("new-dir");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirCreate {
                path: path.to_path_buf(),
                all: true,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], ResponseData::Ok),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that the directory was actually created
        assert!(path.exists(), "Directory not created");
    }

    #[tokio::test]
    async fn remove_should_send_error_on_failure() {
        todo!();
    }

    #[tokio::test]
    async fn remove_should_support_deleting_a_directory() {
        todo!();
    }

    #[tokio::test]
    async fn remove_should_delete_nonempty_directory_if_force_is_true() {
        todo!();
    }

    #[tokio::test]
    async fn remove_should_support_deleting_a_single_file() {
        todo!();
    }

    #[tokio::test]
    async fn copy_should_send_error_on_failure() {
        todo!();
    }

    #[tokio::test]
    async fn copy_should_support_copying_an_entire_directory() {
        todo!();
    }

    #[tokio::test]
    async fn copy_should_support_copying_a_single_file() {
        todo!();
    }

    #[tokio::test]
    async fn rename_should_send_error_on_failure() {
        todo!();
    }

    #[tokio::test]
    async fn rename_should_support_renaming_an_entire_directory() {
        todo!();
    }

    #[tokio::test]
    async fn rename_should_support_renaming_a_single_file() {
        todo!();
    }

    #[tokio::test]
    async fn exists_should_send_error_on_failure() {
        todo!();
    }

    #[tokio::test]
    async fn exists_should_send_true_if_path_exists() {
        todo!();
    }

    #[tokio::test]
    async fn exists_should_send_false_if_path_does_not_exist() {
        todo!();
    }

    #[tokio::test]
    async fn metadata_should_send_error_on_failure() {
        todo!();
    }

    #[tokio::test]
    async fn metadata_should_send_back_metadata_on_file_if_exists() {
        todo!();
    }

    #[tokio::test]
    async fn metadata_should_send_back_metadata_on_dir_if_exists() {
        todo!();
    }

    #[tokio::test]
    async fn metadata_should_include_canonicalized_path_if_flag_specified() {
        todo!();
    }

    #[tokio::test]
    async fn proc_run_should_send_error_on_failure() {
        todo!();
    }

    #[tokio::test]
    async fn proc_run_should_send_back_proc_start_on_success() {
        todo!();
    }

    #[tokio::test]
    async fn proc_run_should_send_back_stdout_periodically_when_available() {
        todo!();
    }

    #[tokio::test]
    async fn proc_run_should_send_back_stderr_periodically_when_available() {
        todo!();
    }

    #[tokio::test]
    async fn proc_run_should_send_back_done_when_proc_finishes() {
        // Make sure to verify that process also removed from state
        todo!();
    }

    #[tokio::test]
    async fn proc_run_should_send_back_done_when_killed() {
        // Make sure to verify that process also removed from state
        todo!();
    }

    #[tokio::test]
    async fn proc_kill_should_send_error_on_failure() {
        // Can verify that if the process is not running, will fail
        todo!();
    }

    #[tokio::test]
    async fn proc_kill_should_send_ok_on_success() {
        // Verify that we trigger sending done
        todo!();
    }

    #[tokio::test]
    async fn proc_stdin_should_send_error_on_failure() {
        // Can verify that if the process is not running, will fail
        todo!();
    }

    #[tokio::test]
    async fn proc_stdin_should_send_ok_on_success() {
        // Verify that we trigger sending stdin to process
        todo!();
    }

    #[tokio::test]
    async fn proc_list_should_send_proc_entry_list() {
        todo!();
    }

    #[tokio::test]
    async fn system_info_should_send_system_info_based_on_binary() {
        todo!();
    }
}
