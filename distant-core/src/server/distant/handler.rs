use crate::{
    constants::{MAX_PIPE_CHUNK_SIZE, READ_PAUSE_MILLIS},
    data::{
        self, DirEntry, FileType, Metadata, Request, RequestData, Response, ResponseData,
        RunningProcess, SystemInfo,
    },
    server::distant::state::{Process, State},
};
use derive_more::{Display, Error, From};
use futures::future;
use log::*;
use std::{
    env,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Arc,
    time::SystemTime,
};
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, oneshot, Mutex, MutexGuard},
};
use walkdir::WalkDir;

type HState = Arc<Mutex<State>>;
type ReplyRet = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;

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

type PostHook = Box<dyn FnOnce(MutexGuard<'_, State>) + Send>;
struct Outgoing {
    data: ResponseData,
    post_hook: Option<PostHook>,
}

impl From<ResponseData> for Outgoing {
    fn from(data: ResponseData) -> Self {
        Self {
            data,
            post_hook: None,
        }
    }
}

/// Processes the provided request, sending replies using the given sender
pub(super) async fn process(
    conn_id: usize,
    state: HState,
    req: Request,
    tx: mpsc::Sender<Response>,
) -> Result<(), mpsc::error::SendError<Response>> {
    async fn inner<F>(
        conn_id: usize,
        state: HState,
        data: RequestData,
        reply: F,
    ) -> Result<Outgoing, ServerError>
    where
        F: FnMut(Vec<ResponseData>) -> ReplyRet + Clone + Send + 'static,
    {
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
            RequestData::Metadata {
                path,
                canonicalize,
                resolve_file_type,
            } => metadata(path, canonicalize, resolve_file_type).await,
            RequestData::ProcRun {
                cmd,
                args,
                detached,
            } => proc_run(conn_id, state, reply, cmd, args, detached).await,
            RequestData::ProcKill { id } => proc_kill(conn_id, state, id).await,
            RequestData::ProcStdin { id, data } => proc_stdin(conn_id, state, id, data).await,
            RequestData::ProcList {} => proc_list(state).await,
            RequestData::SystemInfo {} => system_info().await,
        }
    }

    let reply = {
        let origin_id = req.id;
        let tenant = req.tenant.clone();
        let tx_2 = tx.clone();
        move |payload: Vec<ResponseData>| -> ReplyRet {
            let tx = tx_2.clone();
            let res = Response::new(tenant.to_string(), origin_id, payload);
            Box::pin(async move { tx.send(res).await.is_ok() })
        }
    };

    // Build up a collection of tasks to run independently
    let mut payload_tasks = Vec::new();
    for data in req.payload {
        let state_2 = Arc::clone(&state);
        let reply_2 = reply.clone();
        payload_tasks.push(tokio::spawn(async move {
            match inner(conn_id, state_2, data, reply_2).await {
                Ok(outgoing) => outgoing,
                Err(x) => Outgoing::from(ResponseData::from(x)),
            }
        }));
    }

    // Collect the results of our tasks into the payload entries
    let mut outgoing: Vec<Outgoing> = future::join_all(payload_tasks)
        .await
        .into_iter()
        .map(|x| match x {
            Ok(outgoing) => outgoing,
            Err(x) => Outgoing::from(ResponseData::from(x)),
        })
        .collect();

    let post_hooks: Vec<PostHook> = outgoing
        .iter_mut()
        .filter_map(|x| x.post_hook.take())
        .collect();

    let payload = outgoing.into_iter().map(|x| x.data).collect();
    let res = Response::new(req.tenant, req.id, payload);

    // Send out our primary response from processing the request
    let result = tx.send(res).await;

    // Invoke all post hooks
    for hook in post_hooks {
        hook(state.lock().await);
    }

    result
}

async fn file_read(path: PathBuf) -> Result<Outgoing, ServerError> {
    Ok(Outgoing::from(ResponseData::Blob {
        data: tokio::fs::read(path).await?,
    }))
}

async fn file_read_text(path: PathBuf) -> Result<Outgoing, ServerError> {
    Ok(Outgoing::from(ResponseData::Text {
        data: tokio::fs::read_to_string(path).await?,
    }))
}

async fn file_write(path: PathBuf, data: impl AsRef<[u8]>) -> Result<Outgoing, ServerError> {
    tokio::fs::write(path, data).await?;
    Ok(Outgoing::from(ResponseData::Ok))
}

async fn file_append(path: PathBuf, data: impl AsRef<[u8]>) -> Result<Outgoing, ServerError> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(data.as_ref()).await?;
    Ok(Outgoing::from(ResponseData::Ok))
}

async fn dir_read(
    path: PathBuf,
    depth: usize,
    absolute: bool,
    canonicalize: bool,
    include_root: bool,
) -> Result<Outgoing, ServerError> {
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

    Ok(Outgoing::from(ResponseData::DirEntries { entries, errors }))
}

async fn dir_create(path: PathBuf, all: bool) -> Result<Outgoing, ServerError> {
    if all {
        tokio::fs::create_dir_all(path).await?;
    } else {
        tokio::fs::create_dir(path).await?;
    }

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn remove(path: PathBuf, force: bool) -> Result<Outgoing, ServerError> {
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

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn copy(src: PathBuf, dst: PathBuf) -> Result<Outgoing, ServerError> {
    let src_metadata = tokio::fs::metadata(src.as_path()).await?;
    if src_metadata.is_dir() {
        // Create the destination directory first, regardless of if anything
        // is in the source directory
        tokio::fs::create_dir_all(dst.as_path()).await?;

        for entry in WalkDir::new(src.as_path())
            .min_depth(1)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                e.file_type().is_file() || e.file_type().is_dir() || e.path_is_symlink()
            })
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

            let dst_path = dst_parent_dir.join(local_src_file_name);

            // Perform copying from entry to destination (if a file/symlink)
            if !entry.file_type().is_dir() {
                tokio::fs::copy(entry.path(), dst_path).await?;

            // Otherwise, if a directory, create it
            } else {
                tokio::fs::create_dir(dst_path).await?;
            }
        }
    } else {
        tokio::fs::copy(src, dst).await?;
    }

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn rename(src: PathBuf, dst: PathBuf) -> Result<Outgoing, ServerError> {
    tokio::fs::rename(src, dst).await?;

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn exists(path: PathBuf) -> Result<Outgoing, ServerError> {
    // Following experimental `std::fs::try_exists`, which checks the error kind of the
    // metadata lookup to see if it is not found and filters accordingly
    Ok(match tokio::fs::metadata(path.as_path()).await {
        Ok(_) => Outgoing::from(ResponseData::Exists { value: true }),
        Err(x) if x.kind() == io::ErrorKind::NotFound => {
            Outgoing::from(ResponseData::Exists { value: false })
        }
        Err(x) => return Err(ServerError::from(x)),
    })
}

async fn metadata(
    path: PathBuf,
    canonicalize: bool,
    resolve_file_type: bool,
) -> Result<Outgoing, ServerError> {
    let metadata = tokio::fs::symlink_metadata(path.as_path()).await?;
    let canonicalized_path = if canonicalize {
        Some(tokio::fs::canonicalize(path.as_path()).await?)
    } else {
        None
    };

    // If asking for resolved file type and current type is symlink, then we want to refresh
    // our metadata to get the filetype for the resolved link
    let file_type = if resolve_file_type && metadata.file_type().is_symlink() {
        tokio::fs::metadata(path).await?.file_type()
    } else {
        metadata.file_type()
    };

    Ok(Outgoing::from(ResponseData::Metadata(Metadata {
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
        file_type: if file_type.is_dir() {
            FileType::Dir
        } else if file_type.is_file() {
            FileType::File
        } else {
            FileType::Symlink
        },
    })))
}

async fn proc_run<F>(
    conn_id: usize,
    state: HState,
    reply: F,
    cmd: String,
    args: Vec<String>,
    detached: bool,
) -> Result<Outgoing, ServerError>
where
    F: FnMut(Vec<ResponseData>) -> ReplyRet + Clone + Send + 'static,
{
    let id = rand::random();

    debug!(
        "<Conn @ {} | Proc {}> Spawning {} {}",
        conn_id,
        id,
        cmd,
        args.join(" ")
    );
    let mut child = Command::new(cmd.to_string())
        .args(args.clone())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    state
        .lock()
        .await
        .push_process(conn_id, Process::new(id, cmd, args, detached));

    let post_hook = Box::new(move |mut state_lock: MutexGuard<'_, State>| {
        // Spawn a task that sends stdout as a response
        let mut stdout = child.stdout.take().unwrap();
        let mut reply_2 = reply.clone();
        let stdout_task = tokio::spawn(async move {
            let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
            loop {
                match stdout.read(&mut buf).await {
                    Ok(n) if n > 0 => match String::from_utf8(buf[..n].to_vec()) {
                        Ok(data) => {
                            let payload = vec![ResponseData::ProcStdout { id, data }];
                            if !reply_2(payload).await {
                                error!("<Conn @ {} | Proc {}> Stdout channel closed", conn_id, id);
                                break;
                            }

                            // Pause to allow buffer to fill up a little bit, avoiding
                            // spamming with a lot of smaller responses
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                READ_PAUSE_MILLIS,
                            ))
                            .await;
                        }
                        Err(x) => {
                            error!(
                                "<Conn @ {} | Proc {}> Invalid data read from stdout pipe: {}",
                                conn_id, id, x
                            );
                            break;
                        }
                    },
                    Ok(_) => break,
                    Err(x) => {
                        error!(
                            "<Conn @ {} | Proc {}> Reading stdout failed: {}",
                            conn_id, id, x
                        );
                        break;
                    }
                }
            }
        });

        // Spawn a task that sends stderr as a response
        let mut stderr = child.stderr.take().unwrap();
        let mut reply_2 = reply.clone();
        let stderr_task = tokio::spawn(async move {
            let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
            loop {
                match stderr.read(&mut buf).await {
                    Ok(n) if n > 0 => match String::from_utf8(buf[..n].to_vec()) {
                        Ok(data) => {
                            let payload = vec![ResponseData::ProcStderr { id, data }];
                            if !reply_2(payload).await {
                                error!("<Conn @ {} | Proc {}> Stderr channel closed", conn_id, id);
                                break;
                            }

                            // Pause to allow buffer to fill up a little bit, avoiding
                            // spamming with a lot of smaller responses
                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                READ_PAUSE_MILLIS,
                            ))
                            .await;
                        }
                        Err(x) => {
                            error!(
                                "<Conn @ {} | Proc {}> Invalid data read from stdout pipe: {}",
                                conn_id, id, x
                            );
                            break;
                        }
                    },
                    Ok(_) => break,
                    Err(x) => {
                        error!(
                            "<Conn @ {} | Proc {}> Reading stderr failed: {}",
                            conn_id, id, x
                        );
                        break;
                    }
                }
            }
        });

        // Spawn a task that sends stdin to the process
        let mut stdin = child.stdin.take().unwrap();
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(1);
        let stdin_task = tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                if let Err(x) = stdin.write_all(line.as_bytes()).await {
                    error!(
                        "<Conn @ {} | Proc {}> Failed to send stdin: {}",
                        conn_id, id, x
                    );
                    break;
                }
            }
        });

        // Spawn a task that waits on the process to exit but can also
        // kill the process when triggered
        let state_2 = Arc::clone(&state);
        let (kill_tx, kill_rx) = oneshot::channel();
        let mut reply_2 = reply.clone();
        let wait_task = tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    debug!(
                        "<Conn @ {} | Proc {}> Completed and waiting on stdout & stderr tasks",
                        conn_id,
                        id,
                    );

                    // Force stdin task to abort if it hasn't exited as there is no
                    // point to sending any more stdin
                    stdin_task.abort();

                    if let Err(x) = stderr_task.await {
                        error!("<Conn @ {} | Proc {}> Join on stderr task failed: {}", conn_id, id, x);
                    }

                    if let Err(x) = stdout_task.await {
                        error!("<Conn @ {} | Proc {}> Join on stdout task failed: {}", conn_id, id, x);
                    }

                    state_2.lock().await.remove_process(conn_id, id);

                    match status {
                        Ok(status) => {
                            let success = status.success();
                            let mut code = status.code();

                            // If we succeeded and have no exit code, automatically populate
                            // with success exit code
                            if success && code.is_none() {
                                code = Some(0);
                            }

                            let payload = vec![ResponseData::ProcDone { id, success, code }];
                            if !reply_2(payload).await {
                                error!(
                                    "<Conn @ {} | Proc {}> Failed to send done",
                                    conn_id,
                                    id,
                                );
                            }
                        }
                        Err(x) => {
                            let payload = vec![ResponseData::from(x)];
                            if !reply_2(payload).await {
                                error!(
                                    "<Conn @ {} | Proc {}> Failed to send error for waiting",
                                    conn_id,
                                    id,
                                );
                            }
                        }
                    }

                },
                _ = kill_rx => {
                    debug!("<Conn @ {} | Proc {}> Killing", conn_id, id);

                    if let Err(x) = child.kill().await {
                        error!("<Conn @ {} | Proc {}> Unable to kill: {}", conn_id, id, x);
                    }

                    // Force stdin task to abort if it hasn't exited as there is no
                    // point to sending any more stdin
                    stdin_task.abort();

                    if let Err(x) = stderr_task.await {
                        error!("<Conn @ {} | Proc {}> Join on stderr task failed: {}", conn_id, id, x);
                    }

                    if let Err(x) = stdout_task.await {
                        error!("<Conn @ {} | Proc {}> Join on stdout task failed: {}", conn_id, id, x);
                    }

                    // Wait for the child after being killed to ensure that it has been cleaned
                    // up at the operating system level
                    if let Err(x) = child.wait().await {
                        error!("<Conn @ {} | Proc {}> Failed to wait after killed: {}", conn_id, id, x);
                    }

                    state_2.lock().await.remove_process(conn_id, id);

                    let payload = vec![ResponseData::ProcDone { id, success: false, code: None }];
                    if !reply_2(payload).await {
                        error!("<Conn @ {} | Proc {}> Failed to send done", conn_id, id);
                    }
                }
            }
        });

        // Update our state with the new process
        if let Some(proc) = state_lock.mut_process(id) {
            proc.initialize(stdin_tx, kill_tx, wait_task);
        }
    });

    debug!(
        "<Conn @ {} | Proc {}> Spawned successfully! Will enter post hook later",
        conn_id, id
    );
    Ok(Outgoing {
        data: ResponseData::ProcStart { id },
        post_hook: Some(post_hook),
    })
}

async fn proc_kill(conn_id: usize, state: HState, id: usize) -> Result<Outgoing, ServerError> {
    if let Some(process) = state.lock().await.processes.remove(&id) {
        if process.kill() {
            return Ok(Outgoing::from(ResponseData::Ok));
        }
    }

    Err(ServerError::IoError(io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!(
            "<Conn @ {} | Proc {}> Unable to send kill signal to process",
            conn_id, id
        ),
    )))
}

async fn proc_stdin(
    conn_id: usize,
    state: HState,
    id: usize,
    data: String,
) -> Result<Outgoing, ServerError> {
    if let Some(process) = state.lock().await.processes.get(&id) {
        if process.send_stdin(data).await {
            return Ok(Outgoing::from(ResponseData::Ok));
        }
    }

    Err(ServerError::IoError(io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!(
            "<Conn @ {} | Proc {}> Unable to send stdin to process",
            conn_id, id,
        ),
    )))
}

async fn proc_list(state: HState) -> Result<Outgoing, ServerError> {
    Ok(Outgoing::from(ResponseData::ProcEntries {
        entries: state
            .lock()
            .await
            .processes
            .values()
            .map(|p| RunningProcess {
                cmd: p.cmd.to_string(),
                args: p.args.clone(),
                detached: p.detached,
                id: p.id,
            })
            .collect(),
    }))
}

async fn system_info() -> Result<Outgoing, ServerError> {
    Ok(Outgoing::from(ResponseData::SystemInfo(SystemInfo {
        family: env::consts::FAMILY.to_string(),
        os: env::consts::OS.to_string(),
        arch: env::consts::ARCH.to_string(),
        current_dir: env::current_dir().unwrap_or_default(),
        main_separator: std::path::MAIN_SEPARATOR,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;
    use once_cell::sync::Lazy;
    use predicates::prelude::*;
    use std::time::Duration;

    static TEMP_SCRIPT_DIR: Lazy<assert_fs::TempDir> =
        Lazy::new(|| assert_fs::TempDir::new().unwrap());
    static SCRIPT_RUNNER: Lazy<String> = Lazy::new(|| String::from("bash"));

    static ECHO_ARGS_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
        let script = TEMP_SCRIPT_DIR.child("echo_args_to_stdout.sh");
        script
            .write_str(indoc::indoc!(
                r#"
                #/usr/bin/env bash
                printf "%s" "$*"
            "#
            ))
            .unwrap();
        script
    });

    static ECHO_ARGS_TO_STDERR_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
        let script = TEMP_SCRIPT_DIR.child("echo_args_to_stderr.sh");
        script
            .write_str(indoc::indoc!(
                r#"
                #/usr/bin/env bash
                printf "%s" "$*" 1>&2
            "#
            ))
            .unwrap();
        script
    });

    static ECHO_STDIN_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
        let script = TEMP_SCRIPT_DIR.child("echo_stdin_to_stdout.sh");
        script
            .write_str(indoc::indoc!(
                r#"
                #/usr/bin/env bash
                while IFS= read; do echo "$REPLY"; done
            "#
            ))
            .unwrap();
        script
    });

    static SLEEP_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
        let script = TEMP_SCRIPT_DIR.child("sleep.sh");
        script
            .write_str(indoc::indoc!(
                r#"
                #!/usr/bin/env bash
                sleep "$1"
            "#
            ))
            .unwrap();
        script
    });

    static DOES_NOT_EXIST_BIN: Lazy<assert_fs::fixture::ChildPath> =
        Lazy::new(|| TEMP_SCRIPT_DIR.child("does_not_exist_bin"));

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

    #[tokio::test]
    async fn file_read_should_send_error_if_fails_to_read_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        let temp = assert_fs::TempDir::new().unwrap();
        let path = temp.child("missing-file").path().to_path_buf();

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

        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");
        file.write_str("some file contents").unwrap();

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

        let temp = assert_fs::TempDir::new().unwrap();
        let path = temp.child("missing-file").path().to_path_buf();

        let req = Request::new("test-tenant", vec![RequestData::FileReadText { path }]);

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

        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");
        file.write_str("some file contents").unwrap();

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
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("dir").child("test-file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileWrite {
                path: file.path().to_path_buf(),
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
        file.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn file_write_should_send_ok_when_successful() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Path should point to a file that does not exist, but all
        // other components leading up to it do
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileWrite {
                path: file.path().to_path_buf(),
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
        file.assert("some text");
    }

    #[tokio::test]
    async fn file_write_text_should_send_error_if_fails_to_write_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("dir").child("test-file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileWriteText {
                path: file.path().to_path_buf(),
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
        file.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn file_write_text_should_send_ok_when_successful() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Path should point to a file that does not exist, but all
        // other components leading up to it do
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileWriteText {
                path: file.path().to_path_buf(),
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
        file.assert("some text");
    }

    #[tokio::test]
    async fn file_append_should_send_error_if_fails_to_create_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("dir").child("test-file");

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
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we didn't actually create the file
        file.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn file_append_should_create_file_if_missing() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Don't create the file directly, but define path
        // where the file should be
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");

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

        // Yield to allow chance to finish appending to file
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Also verify that we actually did create to the file
        file.assert("some extra contents");
    }

    #[tokio::test]
    async fn file_append_should_send_ok_when_successful() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary file and fill it with some contents
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");
        file.write_str("some file contents").unwrap();

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

        // Yield to allow chance to finish appending to file
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Also verify that we actually did append to the file
        file.assert("some file contentssome extra contents");
    }

    #[tokio::test]
    async fn file_append_text_should_send_error_if_fails_to_create_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary path and add to it to ensure that there are
        // extra components that don't exist to cause writing to fail
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("dir").child("test-file");

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
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Also verify that we didn't actually create the file
        file.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn file_append_text_should_create_file_if_missing() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Don't create the file directly, but define path
        // where the file should be
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::FileAppendText {
                path: file.path().to_path_buf(),
                text: "some extra contents".to_string(),
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

        // Yield to allow chance to finish appending to file
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Also verify that we actually did create to the file
        file.assert("some extra contents");
    }

    #[tokio::test]
    async fn file_append_text_should_send_ok_when_successful() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create a temporary file and fill it with some contents
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test-file");
        file.write_str("some file contents").unwrap();

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

        // Yield to allow chance to finish appending to file
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Also verify that we actually did append to the file
        file.assert("some file contentssome extra contents");
    }

    #[tokio::test]
    async fn dir_read_should_send_error_if_directory_does_not_exist() {
        let (conn_id, state, tx, mut rx) = setup(1);

        let temp = assert_fs::TempDir::new().unwrap();
        let dir = temp.child("test-dir");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirRead {
                path: dir.path().to_path_buf(),
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
    // /root/link1 -> /root/sub1/file2
    // /root/sub1/
    // /root/sub1/file2
    async fn setup_dir() -> assert_fs::TempDir {
        let root_dir = assert_fs::TempDir::new().unwrap();
        root_dir.child("file1").touch().unwrap();

        let sub1 = root_dir.child("sub1");
        sub1.create_dir_all().unwrap();

        let file2 = sub1.child("file2");
        file2.touch().unwrap();

        let link1 = root_dir.child("link1");
        link1.symlink_to_file(file2.path()).unwrap();

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
                assert_eq!(entries.len(), 3, "Wrong number of entries found");

                assert_eq!(entries[0].file_type, FileType::File);
                assert_eq!(entries[0].path, Path::new("file1"));
                assert_eq!(entries[0].depth, 1);

                assert_eq!(entries[1].file_type, FileType::Symlink);
                assert_eq!(entries[1].path, Path::new("link1"));
                assert_eq!(entries[1].depth, 1);

                assert_eq!(entries[2].file_type, FileType::Dir);
                assert_eq!(entries[2].path, Path::new("sub1"));
                assert_eq!(entries[2].depth, 1);
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
                assert_eq!(entries.len(), 4, "Wrong number of entries found");

                assert_eq!(entries[0].file_type, FileType::File);
                assert_eq!(entries[0].path, Path::new("file1"));
                assert_eq!(entries[0].depth, 1);

                assert_eq!(entries[1].file_type, FileType::Symlink);
                assert_eq!(entries[1].path, Path::new("link1"));
                assert_eq!(entries[1].depth, 1);

                assert_eq!(entries[2].file_type, FileType::Dir);
                assert_eq!(entries[2].path, Path::new("sub1"));
                assert_eq!(entries[2].depth, 1);

                assert_eq!(entries[3].file_type, FileType::File);
                assert_eq!(entries[3].path, Path::new("sub1").join("file2"));
                assert_eq!(entries[3].depth, 2);
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
                assert_eq!(entries.len(), 4, "Wrong number of entries found");

                // NOTE: Root entry is always absolute, resolved path
                assert_eq!(entries[0].file_type, FileType::Dir);
                assert_eq!(entries[0].path, root_dir.path().canonicalize().unwrap());
                assert_eq!(entries[0].depth, 0);

                assert_eq!(entries[1].file_type, FileType::File);
                assert_eq!(entries[1].path, Path::new("file1"));
                assert_eq!(entries[1].depth, 1);

                assert_eq!(entries[2].file_type, FileType::Symlink);
                assert_eq!(entries[2].path, Path::new("link1"));
                assert_eq!(entries[2].depth, 1);

                assert_eq!(entries[3].file_type, FileType::Dir);
                assert_eq!(entries[3].path, Path::new("sub1"));
                assert_eq!(entries[3].depth, 1);
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
                assert_eq!(entries.len(), 3, "Wrong number of entries found");
                let root_path = root_dir.path().canonicalize().unwrap();

                assert_eq!(entries[0].file_type, FileType::File);
                assert_eq!(entries[0].path, root_path.join("file1"));
                assert_eq!(entries[0].depth, 1);

                assert_eq!(entries[1].file_type, FileType::Symlink);
                assert_eq!(entries[1].path, root_path.join("link1"));
                assert_eq!(entries[1].depth, 1);

                assert_eq!(entries[2].file_type, FileType::Dir);
                assert_eq!(entries[2].path, root_path.join("sub1"));
                assert_eq!(entries[2].depth, 1);
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn dir_read_should_support_returning_canonicalized_paths() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Create directory with some nested items
        let root_dir = setup_dir().await;

        let req = Request::new(
            "test-tenant",
            vec![RequestData::DirRead {
                path: root_dir.path().to_path_buf(),
                depth: 1,
                absolute: false,
                canonicalize: true,
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

                // Symlink should be resolved from $ROOT/link1 -> $ROOT/sub1/file2
                assert_eq!(entries[1].file_type, FileType::Symlink);
                assert_eq!(entries[1].path, Path::new("sub1").join("file2"));
                assert_eq!(entries[1].depth, 1);

                assert_eq!(entries[2].file_type, FileType::Dir);
                assert_eq!(entries[2].path, Path::new("sub1"));
                assert_eq!(entries[2].depth, 1);
            }
            x => panic!("Unexpected response: {:?}", x),
        }
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
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("missing-file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Remove {
                path: file.path().to_path_buf(),
                force: false,
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

        // Also, verify that path does not exist
        file.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn remove_should_support_deleting_a_directory() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let dir = temp.child("dir");
        dir.create_dir_all().unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Remove {
                path: dir.path().to_path_buf(),
                force: false,
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

        // Also, verify that path does not exist
        dir.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn remove_should_delete_nonempty_directory_if_force_is_true() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let dir = temp.child("dir");
        dir.create_dir_all().unwrap();
        dir.child("file").touch().unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Remove {
                path: dir.path().to_path_buf(),
                force: true,
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

        // Also, verify that path does not exist
        dir.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn remove_should_support_deleting_a_single_file() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("some-file");
        file.touch().unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Remove {
                path: file.path().to_path_buf(),
                force: false,
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

        // Also, verify that path does not exist
        file.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn copy_should_send_error_on_failure() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        let dst = temp.child("dst");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
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

        // Also, verify that destination does not exist
        dst.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn copy_should_support_copying_an_entire_directory() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();

        let src = temp.child("src");
        src.create_dir_all().unwrap();
        let src_file = src.child("file");
        src_file.write_str("some contents").unwrap();

        let dst = temp.child("dst");
        let dst_file = dst.child("file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
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

        // Verify that we have source and destination directories and associated contents
        src.assert(predicate::path::is_dir());
        src_file.assert(predicate::path::is_file());
        dst.assert(predicate::path::is_dir());
        dst_file.assert(predicate::path::eq_file(src_file.path()));
    }

    #[tokio::test]
    async fn copy_should_support_copying_an_empty_directory() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        src.create_dir_all().unwrap();
        let dst = temp.child("dst");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
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

        // Verify that we still have source and destination directories
        src.assert(predicate::path::is_dir());
        dst.assert(predicate::path::is_dir());
    }

    #[tokio::test]
    async fn copy_should_support_copying_a_directory_that_only_contains_directories() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();

        let src = temp.child("src");
        src.create_dir_all().unwrap();
        let src_dir = src.child("dir");
        src_dir.create_dir_all().unwrap();

        let dst = temp.child("dst");
        let dst_dir = dst.child("dir");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
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

        // Verify that we have source and destination directories and associated contents
        src.assert(predicate::path::is_dir().name("src"));
        src_dir.assert(predicate::path::is_dir().name("src/dir"));
        dst.assert(predicate::path::is_dir().name("dst"));
        dst_dir.assert(predicate::path::is_dir().name("dst/dir"));
    }

    #[tokio::test]
    async fn copy_should_support_copying_a_single_file() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        src.write_str("some text").unwrap();
        let dst = temp.child("dst");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
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

        // Verify that we still have source and that destination has source's contents
        src.assert(predicate::path::is_file());
        dst.assert(predicate::path::eq_file(src.path()));
    }

    #[tokio::test]
    async fn rename_should_send_error_on_failure() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        let dst = temp.child("dst");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Rename {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
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

        // Also, verify that destination does not exist
        dst.assert(predicate::path::missing());
    }

    #[tokio::test]
    async fn rename_should_support_renaming_an_entire_directory() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();

        let src = temp.child("src");
        src.create_dir_all().unwrap();
        let src_file = src.child("file");
        src_file.write_str("some contents").unwrap();

        let dst = temp.child("dst");
        let dst_file = dst.child("file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Rename {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
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

        // Verify that we moved the contents
        src.assert(predicate::path::missing());
        src_file.assert(predicate::path::missing());
        dst.assert(predicate::path::is_dir());
        dst_file.assert("some contents");
    }

    #[tokio::test]
    async fn rename_should_support_renaming_a_single_file() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let src = temp.child("src");
        src.write_str("some text").unwrap();
        let dst = temp.child("dst");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Rename {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
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

        // Verify that we moved the file
        src.assert(predicate::path::missing());
        dst.assert("some text");
    }

    #[tokio::test]
    async fn exists_should_send_true_if_path_exists() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.touch().unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Exists {
                path: file.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert_eq!(res.payload[0], ResponseData::Exists { value: true });
    }

    #[tokio::test]
    async fn exists_should_send_false_if_path_does_not_exist() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Exists {
                path: file.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert_eq!(res.payload[0], ResponseData::Exists { value: false });
    }

    #[tokio::test]
    async fn metadata_should_send_error_on_failure() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Metadata {
                path: file.path().to_path_buf(),
                canonicalize: false,
                resolve_file_type: false,
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

    #[tokio::test]
    async fn metadata_should_send_back_metadata_on_file_if_exists() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Metadata {
                path: file.path().to_path_buf(),
                canonicalize: false,
                resolve_file_type: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(
                res.payload[0],
                ResponseData::Metadata(Metadata {
                    canonicalized_path: None,
                    file_type: FileType::File,
                    len: 9,
                    readonly: false,
                    ..
                })
            ),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn metadata_should_send_back_metadata_on_dir_if_exists() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let dir = temp.child("dir");
        dir.create_dir_all().unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Metadata {
                path: dir.path().to_path_buf(),
                canonicalize: false,
                resolve_file_type: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(
                res.payload[0],
                ResponseData::Metadata(Metadata {
                    canonicalized_path: None,
                    file_type: FileType::Dir,
                    readonly: false,
                    ..
                })
            ),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn metadata_should_send_back_metadata_on_symlink_if_exists() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_file(file.path()).unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Metadata {
                path: symlink.path().to_path_buf(),
                canonicalize: false,
                resolve_file_type: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(
                res.payload[0],
                ResponseData::Metadata(Metadata {
                    canonicalized_path: None,
                    file_type: FileType::Symlink,
                    readonly: false,
                    ..
                })
            ),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn metadata_should_include_canonicalized_path_if_flag_specified() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_file(file.path()).unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Metadata {
                path: symlink.path().to_path_buf(),
                canonicalize: true,
                resolve_file_type: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::Metadata(Metadata {
                canonicalized_path: Some(path),
                file_type: FileType::Symlink,
                readonly: false,
                ..
            }) => assert_eq!(
                path,
                &file.path().canonicalize().unwrap(),
                "Symlink canonicalized path does not match referenced file"
            ),
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn metadata_should_resolve_file_type_of_symlink_if_flag_specified() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let symlink = temp.child("link");
        symlink.symlink_to_file(file.path()).unwrap();

        let req = Request::new(
            "test-tenant",
            vec![RequestData::Metadata {
                path: symlink.path().to_path_buf(),
                canonicalize: false,
                resolve_file_type: true,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            ResponseData::Metadata(Metadata {
                file_type: FileType::File,
                ..
            }) => {}
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn proc_run_should_send_error_on_failure() {
        let (conn_id, state, tx, mut rx) = setup(1);

        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcRun {
                cmd: DOES_NOT_EXIST_BIN.to_str().unwrap().to_string(),
                args: Vec::new(),
                detached: false,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(&res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn proc_run_should_send_back_proc_start_on_success() {
        let (conn_id, state, tx, mut rx) = setup(1);

        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcRun {
                cmd: SCRIPT_RUNNER.to_string(),
                args: vec![ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string()],
                detached: false,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(&res.payload[0], ResponseData::ProcStart { .. }),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn proc_run_should_send_back_stdout_periodically_when_available() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Run a program that echoes to stdout
        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcRun {
                cmd: SCRIPT_RUNNER.to_string(),
                args: vec![
                    ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string(),
                    String::from("some stdout"),
                ],
                detached: false,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(&res.payload[0], ResponseData::ProcStart { .. }),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Gather two additional responses:
        //
        // 1. An indirect response for stdout
        // 2. An indirect response that is proc completing
        //
        // Note that order is not a guarantee, so we have to check that
        // we get one of each type of response
        let res1 = rx.recv().await.expect("Missing first response");
        let res2 = rx.recv().await.expect("Missing second response");

        let mut got_stdout = false;
        let mut got_done = false;

        let mut check_res = |res: &Response| {
            assert_eq!(res.payload.len(), 1, "Wrong payload size");
            match &res.payload[0] {
                ResponseData::ProcStdout { data, .. } => {
                    assert_eq!(data, "some stdout", "Got wrong stdout");
                    got_stdout = true;
                }
                ResponseData::ProcDone { success, .. } => {
                    assert!(success, "Process should have completed successfully");
                    got_done = true;
                }
                x => panic!("Unexpected response: {:?}", x),
            }
        };

        check_res(&res1);
        check_res(&res2);
        assert!(got_stdout, "Missing stdout response");
        assert!(got_done, "Missing done response");
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn proc_run_should_send_back_stderr_periodically_when_available() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Run a program that echoes to stderr
        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcRun {
                cmd: SCRIPT_RUNNER.to_string(),
                args: vec![
                    ECHO_ARGS_TO_STDERR_SH.to_str().unwrap().to_string(),
                    String::from("some stderr"),
                ],
                detached: false,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(&res.payload[0], ResponseData::ProcStart { .. }),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Gather two additional responses:
        //
        // 1. An indirect response for stderr
        // 2. An indirect response that is proc completing
        //
        // Note that order is not a guarantee, so we have to check that
        // we get one of each type of response
        let res1 = rx.recv().await.expect("Missing first response");
        let res2 = rx.recv().await.expect("Missing second response");

        let mut got_stderr = false;
        let mut got_done = false;

        let mut check_res = |res: &Response| {
            assert_eq!(res.payload.len(), 1, "Wrong payload size");
            match &res.payload[0] {
                ResponseData::ProcStderr { data, .. } => {
                    assert_eq!(data, "some stderr", "Got wrong stderr");
                    got_stderr = true;
                }
                ResponseData::ProcDone { success, .. } => {
                    assert!(success, "Process should have completed successfully");
                    got_done = true;
                }
                x => panic!("Unexpected response: {:?}", x),
            }
        };

        check_res(&res1);
        check_res(&res2);
        assert!(got_stderr, "Missing stderr response");
        assert!(got_done, "Missing done response");
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn proc_run_should_clear_process_from_state_when_done() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Run a program that ends after a little bit
        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcRun {
                cmd: SCRIPT_RUNNER.to_string(),
                args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("0.1")],
                detached: false,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        let id = match &res.payload[0] {
            ResponseData::ProcStart { id } => *id,
            x => panic!("Unexpected response: {:?}", x),
        };

        // Verify that the state has the process
        assert!(
            state.lock().await.processes.contains_key(&id),
            "Process {} not in state",
            id
        );

        // Wait for process to finish
        let _ = rx.recv().await.unwrap();

        // Verify that the state was cleared
        assert!(
            !state.lock().await.processes.contains_key(&id),
            "Process {} still in state",
            id
        );
    }

    #[tokio::test]
    async fn proc_run_should_clear_process_from_state_when_killed() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Run a program that ends slowly
        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcRun {
                cmd: SCRIPT_RUNNER.to_string(),
                args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")],
                detached: false,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        let id = match &res.payload[0] {
            ResponseData::ProcStart { id } => *id,
            x => panic!("Unexpected response: {:?}", x),
        };

        // Verify that the state has the process
        assert!(
            state.lock().await.processes.contains_key(&id),
            "Process {} not in state",
            id
        );

        // Send kill signal
        let req = Request::new("test-tenant", vec![RequestData::ProcKill { id }]);
        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        // Wait for two responses, a kill confirmation and the done
        let _ = rx.recv().await.unwrap();
        let _ = rx.recv().await.unwrap();

        // Verify that the state was cleared
        assert!(
            !state.lock().await.processes.contains_key(&id),
            "Process {} still in state",
            id
        );
    }

    #[tokio::test]
    async fn proc_kill_should_send_error_on_failure() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Send kill to a non-existent process
        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcKill { id: 0xDEADBEEF }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        // Verify that we get an error
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn proc_kill_should_send_ok_and_done_responses_on_success() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // First, run a program that sits around (sleep for 1 second)
        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcRun {
                cmd: SCRIPT_RUNNER.to_string(),
                args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")],
                detached: false,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        // Second, grab the id of the started process
        let id = match &res.payload[0] {
            ResponseData::ProcStart { id } => *id,
            x => panic!("Unexpected response: {:?}", x),
        };

        // Third, send kill for process
        let req = Request::new("test-tenant", vec![RequestData::ProcKill { id }]);

        // NOTE: We cannot let the state get dropped as it results in killing
        //       the child process automatically; so, we clone another reference here
        process(conn_id, Arc::clone(&state), req, tx).await.unwrap();

        // Fourth, gather two responses:
        //
        // 1. A direct response saying that received (ok)
        // 2. An indirect response that is proc completing
        //
        // Note that order is not a guarantee, so we have to check that
        // we get one of each type of response
        let res1 = rx.recv().await.expect("Missing first response");
        let res2 = rx.recv().await.expect("Missing second response");

        let mut got_ok = false;
        let mut got_done = false;

        let mut check_res = |res: &Response| {
            assert_eq!(res.payload.len(), 1, "Wrong payload size");
            match &res.payload[0] {
                ResponseData::Ok => got_ok = true,
                ResponseData::ProcDone { success, .. } => {
                    assert!(!success, "Process should not have completed successfully");
                    got_done = true;
                }
                x => panic!("Unexpected response: {:?}", x),
            }
        };

        check_res(&res1);
        check_res(&res2);
        assert!(got_ok, "Missing ok response");
        assert!(got_done, "Missing done response");
    }

    #[tokio::test]
    async fn proc_stdin_should_send_error_on_failure() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Send stdin to a non-existent process
        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcStdin {
                id: 0xDEADBEEF,
                data: String::from("some input"),
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        // Verify that we get an error
        assert!(
            matches!(res.payload[0], ResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn proc_stdin_should_send_ok_on_success_and_properly_send_stdin_to_process() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // First, run a program that listens for stdin
        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcRun {
                cmd: SCRIPT_RUNNER.to_string(),
                args: vec![ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap().to_string()],
                detached: false,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        // Second, grab the id of the started process
        let id = match &res.payload[0] {
            ResponseData::ProcStart { id } => *id,
            x => panic!("Unexpected response: {:?}", x),
        };

        // Third, send stdin to the remote process
        let req = Request::new(
            "test-tenant",
            vec![RequestData::ProcStdin {
                id,
                data: String::from("hello world\n"),
            }],
        );

        // NOTE: We cannot let the state get dropped as it results in killing
        //       the child process; so, we clone another reference here
        process(conn_id, Arc::clone(&state), req, tx).await.unwrap();

        // Fourth, gather two responses:
        //
        // 1. A direct response to processing the stdin
        // 2. An indirect response that is stdout from echoing our stdin
        //
        // Note that order is not a guarantee, so we have to check that
        // we get one of each type of response
        let res1 = rx.recv().await.expect("Missing first response");
        let res2 = rx.recv().await.expect("Missing second response");

        let mut got_ok = false;
        let mut got_stdout = false;

        let mut check_res = |res: &Response| {
            assert_eq!(res.payload.len(), 1, "Wrong payload size");
            match &res.payload[0] {
                ResponseData::Ok => got_ok = true,
                ResponseData::ProcStdout { data, .. } => {
                    assert_eq!(data, "hello world\n", "Mirrored data didn't match");
                    got_stdout = true;
                }
                x => panic!("Unexpected response: {:?}", x),
            }
        };

        check_res(&res1);
        check_res(&res2);
        assert!(got_ok, "Missing ok response");
        assert!(got_stdout, "Missing mirrored stdin response");
    }

    #[tokio::test]
    async fn proc_list_should_send_proc_entry_list() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Run a process and get the list that includes that process
        // at the same time (using sleep of 1 second)
        let req = Request::new(
            "test-tenant",
            vec![
                RequestData::ProcRun {
                    cmd: SCRIPT_RUNNER.to_string(),
                    args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")],
                    detached: false,
                },
                RequestData::ProcList {},
            ],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 2, "Wrong payload size");

        // Grab the id of the started process
        let id = match &res.payload[0] {
            ResponseData::ProcStart { id } => *id,
            x => panic!("Unexpected response: {:?}", x),
        };

        // Verify our process shows up in our entry list
        assert_eq!(
            res.payload[1],
            ResponseData::ProcEntries {
                entries: vec![RunningProcess {
                    cmd: SCRIPT_RUNNER.to_string(),
                    args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")],
                    detached: false,
                    id,
                }],
            },
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn system_info_should_send_system_info_based_on_binary() {
        let (conn_id, state, tx, mut rx) = setup(1);

        let req = Request::new("test-tenant", vec![RequestData::SystemInfo {}]);

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert_eq!(
            res.payload[0],
            ResponseData::SystemInfo(SystemInfo {
                family: env::consts::FAMILY.to_string(),
                os: env::consts::OS.to_string(),
                arch: env::consts::ARCH.to_string(),
                current_dir: env::current_dir().unwrap_or_default(),
                main_separator: std::path::MAIN_SEPARATOR,
            }),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }
}
