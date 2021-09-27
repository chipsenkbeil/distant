use async_compat::CompatExt;
use distant_core::{
    data::{DirEntry, Error as DistantError, FileType, RunningProcess},
    Request, RequestData, Response, ResponseData,
};
use futures::future;
use log::*;
use std::{
    collections::HashMap,
    future::Future,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio::sync::{mpsc, Mutex};
use wezterm_ssh::{Child, ExecResult, OpenFileType, OpenOptions, Session as WezSession, WriteMode};

const MAX_PIPE_CHUNK_SIZE: usize = 8192;
const READ_PAUSE_MILLIS: u64 = 50;

fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::Other, err)
}

#[derive(Default)]
pub(crate) struct State {
    processes: HashMap<usize, Process>,
}

struct Process {
    id: usize,
    cmd: String,
    args: Vec<String>,
    stdin_tx: mpsc::Sender<String>,
    kill_tx: mpsc::Sender<()>,
}

type ReplyRet = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;

type PostHook = Box<dyn FnOnce() + Send>;
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
    session: WezSession,
    state: Arc<Mutex<State>>,
    req: Request,
    tx: mpsc::Sender<Response>,
) -> Result<(), mpsc::error::SendError<Response>> {
    async fn inner<F>(
        session: WezSession,
        state: Arc<Mutex<State>>,
        data: RequestData,
        reply: F,
    ) -> io::Result<Outgoing>
    where
        F: FnMut(Vec<ResponseData>) -> ReplyRet + Clone + Send + 'static,
    {
        match data {
            RequestData::FileRead { path } => file_read(session, path).await,
            RequestData::FileReadText { path } => file_read_text(session, path).await,
            RequestData::FileWrite { path, data } => file_write(session, path, data).await,
            RequestData::FileWriteText { path, text } => file_write(session, path, text).await,
            RequestData::FileAppend { path, data } => file_append(session, path, data).await,
            RequestData::FileAppendText { path, text } => file_append(session, path, text).await,
            RequestData::DirRead {
                path,
                depth,
                absolute,
                canonicalize,
                include_root,
            } => dir_read(session, path, depth, absolute, canonicalize, include_root).await,
            RequestData::DirCreate { path, all } => dir_create(session, path, all).await,
            RequestData::Remove { path, force } => remove(session, path, force).await,
            RequestData::Copy { src, dst } => copy(session, src, dst).await,
            RequestData::Rename { src, dst } => rename(session, src, dst).await,
            RequestData::Exists { path } => exists(session, path).await,
            RequestData::Metadata {
                path,
                canonicalize,
                resolve_file_type,
            } => metadata(session, path, canonicalize, resolve_file_type).await,
            RequestData::ProcRun { cmd, args } => proc_run(session, state, reply, cmd, args).await,
            RequestData::ProcKill { id } => proc_kill(session, state, id).await,
            RequestData::ProcStdin { id, data } => proc_stdin(session, state, id, data).await,
            RequestData::ProcList {} => proc_list(session, state).await,
            RequestData::SystemInfo {} => system_info(session).await,
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
        let session = session.clone();
        payload_tasks.push(tokio::spawn(async move {
            match inner(session, state_2, data, reply_2).await {
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
        hook();
    }

    result
}

async fn file_read(session: WezSession, path: PathBuf) -> io::Result<Outgoing> {
    use smol::io::AsyncReadExt;
    let mut file = session
        .sftp()
        .open(path)
        .compat()
        .await
        .map_err(to_other_error)?;

    let mut contents = String::new();
    file.read_to_string(&mut contents).compat().await?;

    Ok(Outgoing::from(ResponseData::Blob {
        data: contents.into_bytes(),
    }))
}

async fn file_read_text(session: WezSession, path: PathBuf) -> io::Result<Outgoing> {
    use smol::io::AsyncReadExt;
    let mut file = session
        .sftp()
        .open(path)
        .compat()
        .await
        .map_err(to_other_error)?;

    let mut contents = String::new();
    file.read_to_string(&mut contents).compat().await?;

    Ok(Outgoing::from(ResponseData::Text { data: contents }))
}

async fn file_write(
    session: WezSession,
    path: PathBuf,
    data: impl AsRef<[u8]>,
) -> io::Result<Outgoing> {
    use smol::io::AsyncWriteExt;
    let mut file = session
        .sftp()
        .create(path)
        .compat()
        .await
        .map_err(to_other_error)?;

    file.write_all(data.as_ref()).compat().await?;

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn file_append(
    session: WezSession,
    path: PathBuf,
    data: impl AsRef<[u8]>,
) -> io::Result<Outgoing> {
    use smol::io::AsyncWriteExt;
    let mut file = session
        .sftp()
        .open_mode(
            path,
            OpenOptions {
                read: false,
                write: Some(WriteMode::Append),
                // Using 644 as this mirrors "ssh <host> touch ..."
                // 644: rw-r--r--
                mode: 0o644,
                ty: OpenFileType::File,
            },
        )
        .compat()
        .await
        .map_err(to_other_error)?;

    file.write_all(data.as_ref()).compat().await?;

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn dir_read(
    session: WezSession,
    path: PathBuf,
    depth: usize,
    absolute: bool,
    canonicalize: bool,
    include_root: bool,
) -> io::Result<Outgoing> {
    let sftp = session.sftp();

    // Canonicalize our provided path to ensure that it is exists, not a loop, and absolute
    let root_path = sftp.realpath(path).compat().await.map_err(to_other_error)?;

    // Build up our entry list
    let mut entries = Vec::new();
    let mut errors = Vec::new();

    let mut to_traverse = vec![DirEntry {
        path: root_path.to_path_buf(),
        file_type: FileType::Dir,
        depth: 0,
    }];

    while let Some(entry) = to_traverse.pop() {
        let is_root = entry.depth == 0;
        let next_depth = entry.depth + 1;
        let ft = entry.file_type;
        let path = if entry.path.is_relative() {
            root_path.join(&entry.path)
        } else {
            entry.path.to_path_buf()
        };

        // Always include any non-root in our traverse list, but only include the
        // root directory if flagged to do so
        if !is_root || include_root {
            entries.push(entry);
        }

        let is_dir = match ft {
            FileType::Dir => true,
            FileType::File => false,
            FileType::Symlink => match sftp.stat(&path).await {
                Ok(stat) => stat.is_dir(),
                Err(x) => {
                    errors.push(DistantError::from(to_other_error(x)));
                    continue;
                }
            },
        };

        // Determine if we continue traversing or stop
        if is_dir && (depth == 0 || next_depth <= depth) {
            match sftp.readdir(&path).compat().await.map_err(to_other_error) {
                Ok(entries) => {
                    for (mut path, stat) in entries {
                        // Canonicalize the path if specified, otherwise just return
                        // the path as is
                        path = if canonicalize {
                            match sftp.realpath(path).compat().await {
                                Ok(path) => path,
                                Err(x) => {
                                    errors.push(DistantError::from(to_other_error(x)));
                                    continue;
                                }
                            }
                        } else {
                            path
                        };

                        // Strip the path of its prefix based if not flagged as absolute
                        if !absolute {
                            // NOTE: In the situation where we canonicalized the path earlier,
                            // there is no guarantee that our root path is still the parent of
                            // the symlink's destination; so, in that case we MUST just return
                            // the path if the strip_prefix fails
                            path = path
                                .strip_prefix(root_path.as_path())
                                .map(Path::to_path_buf)
                                .unwrap_or(path);
                        };

                        let ft = stat.ty;
                        to_traverse.push(DirEntry {
                            path,
                            file_type: if ft.is_dir() {
                                FileType::Dir
                            } else if ft.is_file() {
                                FileType::File
                            } else {
                                FileType::Symlink
                            },
                            depth: next_depth,
                        });
                    }
                }
                Err(x) if is_root => return Err(io::Error::new(io::ErrorKind::Other, x)),
                Err(x) => errors.push(DistantError::from(x)),
            }
        }
    }

    // Sort entries by filename
    entries.sort_unstable_by_key(|e| e.path.to_path_buf());

    Ok(Outgoing::from(ResponseData::DirEntries { entries, errors }))
}

async fn dir_create(session: WezSession, path: PathBuf, all: bool) -> io::Result<Outgoing> {
    let sftp = session.sftp();

    // Makes the immediate directory, failing if given a path with missing components
    async fn mkdir(sftp: &wezterm_ssh::Sftp, path: PathBuf) -> io::Result<()> {
        // Using 755 as this mirrors "ssh <host> mkdir ..."
        // 755: rwxr-xr-x
        sftp.mkdir(path, 0o755)
            .compat()
            .await
            .map_err(to_other_error)
    }

    if all {
        // Keep trying to create a directory, moving up to parent each time a failure happens
        let mut failed_paths = Vec::new();
        let mut cur_path = path.as_path();
        loop {
            let failed = mkdir(&sftp, cur_path.to_path_buf()).await.is_err();
            if failed {
                failed_paths.push(cur_path);
                if let Some(path) = cur_path.parent() {
                    cur_path = path;
                } else {
                    return Err(io::Error::from(io::ErrorKind::PermissionDenied));
                }
            } else {
                break;
            }
        }

        // Now that we've successfully created a parent component (or the directory), proceed
        // to attempt to create each failed directory
        while let Some(path) = failed_paths.pop() {
            mkdir(&sftp, path.to_path_buf()).await?;
        }
    } else {
        mkdir(&sftp, path).await?;
    }

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn remove(session: WezSession, path: PathBuf, force: bool) -> io::Result<Outgoing> {
    let sftp = session.sftp();

    // Determine if we are dealing with a file or directory
    let stat = sftp
        .stat(path.to_path_buf())
        .compat()
        .await
        .map_err(to_other_error)?;

    // If a file or symlink, we just unlink (easy)
    if stat.is_file() || stat.is_symlink() {
        sftp.unlink(path)
            .compat()
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::PermissionDenied, x))?;
    // If directory and not forcing, we just rmdir (easy)
    } else if !force {
        sftp.rmdir(path)
            .compat()
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::PermissionDenied, x))?;
    // Otherwise, we need to find all files and directories, keep track of their depth, and
    // then attempt to remove them all
    } else {
        let mut entries = Vec::new();
        let mut to_traverse = vec![DirEntry {
            path,
            file_type: FileType::Dir,
            depth: 0,
        }];

        // Collect all entries within directory
        while let Some(entry) = to_traverse.pop() {
            if entry.file_type == FileType::Dir {
                let path = entry.path.to_path_buf();
                let depth = entry.depth;

                entries.push(entry);

                for (path, stat) in sftp.readdir(path).await.map_err(to_other_error)? {
                    to_traverse.push(DirEntry {
                        path,
                        file_type: if stat.is_dir() {
                            FileType::Dir
                        } else if stat.is_file() {
                            FileType::File
                        } else {
                            FileType::Symlink
                        },
                        depth: depth + 1,
                    });
                }
            } else {
                entries.push(entry);
            }
        }

        // Sort by depth such that deepest are last as we will be popping
        // off entries from end to remove first
        entries.sort_unstable_by_key(|e| e.depth);

        while let Some(entry) = entries.pop() {
            if entry.file_type == FileType::Dir {
                sftp.rmdir(entry.path)
                    .compat()
                    .await
                    .map_err(|x| io::Error::new(io::ErrorKind::PermissionDenied, x))?;
            } else {
                sftp.unlink(entry.path)
                    .compat()
                    .await
                    .map_err(|x| io::Error::new(io::ErrorKind::PermissionDenied, x))?;
            }
        }
    }

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn copy(session: WezSession, src: PathBuf, dst: PathBuf) -> io::Result<Outgoing> {
    // NOTE: SFTP does not provide a remote-to-remote copy method, so we instead execute
    //       a program and hope that it applies, starting with the Unix/BSD/GNU cp method
    //       and switch to Window's xcopy if the former fails

    // Unix cp -R <src> <dst>
    let unix_result = session
        .exec(&format!("cp -R {:?} {:?}", src, dst), None)
        .compat()
        .await;

    let failed = unix_result.is_err() || {
        let exit_status = unix_result.unwrap().child.async_wait().compat().await;
        exit_status.is_err() || !exit_status.unwrap().success()
    };

    // Windows xcopy <src> <dst> /s /e
    if failed {
        let exit_status = session
            .exec(&format!("xcopy {:?} {:?} /s /e", src, dst), None)
            .compat()
            .await
            .map_err(to_other_error)?
            .child
            .async_wait()
            .compat()
            .await
            .map_err(to_other_error)?;

        if !exit_status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Unix and windows copy commands failed",
            ));
        }
    }

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn rename(session: WezSession, src: PathBuf, dst: PathBuf) -> io::Result<Outgoing> {
    session
        .sftp()
        .rename(src, dst, Default::default())
        .compat()
        .await
        .map_err(to_other_error)?;

    Ok(Outgoing::from(ResponseData::Ok))
}

async fn exists(session: WezSession, path: PathBuf) -> io::Result<Outgoing> {
    // NOTE: SFTP does not provide a means to check if a path exists that can be performed
    // separately from getting permission errors; so, we just assume any error means that the path
    // does not exist
    let exists = session.sftp().lstat(path).compat().await.is_ok();

    Ok(Outgoing::from(ResponseData::Exists(exists)))
}

async fn metadata(
    session: WezSession,
    path: PathBuf,
    canonicalize: bool,
    resolve_file_type: bool,
) -> io::Result<Outgoing> {
    let sftp = session.sftp();
    let canonicalized_path = if canonicalize {
        Some(
            sftp.realpath(path.to_path_buf())
                .compat()
                .await
                .map_err(to_other_error)?,
        )
    } else {
        None
    };

    let stat = if resolve_file_type {
        sftp.stat(path).compat().await.map_err(to_other_error)?
    } else {
        sftp.lstat(path).compat().await.map_err(to_other_error)?
    };

    let file_type = if stat.is_dir() {
        FileType::Dir
    } else if stat.is_file() {
        FileType::File
    } else {
        FileType::Symlink
    };

    Ok(Outgoing::from(ResponseData::Metadata {
        canonicalized_path,
        file_type,
        len: stat.size.unwrap_or_default(),
        // Check that owner, group, or other has write permission (if not, then readonly)
        readonly: stat.is_readonly(),
        accessed: stat.accessed.map(u128::from),
        modified: stat.modified.map(u128::from),
        created: None,
    }))
}

async fn proc_run<F>(
    session: WezSession,
    state: Arc<Mutex<State>>,
    reply: F,
    cmd: String,
    args: Vec<String>,
) -> io::Result<Outgoing>
where
    F: FnMut(Vec<ResponseData>) -> ReplyRet + Clone + Send + 'static,
{
    let id = rand::random();
    let cmd_string = format!("{} {}", cmd, args.join(" "));

    let ExecResult {
        mut stdin,
        mut stdout,
        mut stderr,
        mut child,
    } = session
        .exec(&cmd_string, None)
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

    let (stdin_tx, mut stdin_rx) = mpsc::channel(1);
    let (kill_tx, mut kill_rx) = mpsc::channel(1);
    state.lock().await.processes.insert(
        id,
        Process {
            id,
            cmd,
            args,
            stdin_tx,
            kill_tx,
        },
    );

    let post_hook = Box::new(move || {
        // Spawn a task that sends stdout as a response
        let mut reply_2 = reply.clone();
        let stdout_task = tokio::spawn(async move {
            let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
            loop {
                match stdout.read(&mut buf) {
                    Ok(n) if n > 0 => match String::from_utf8(buf[..n].to_vec()) {
                        Ok(data) => {
                            let payload = vec![ResponseData::ProcStdout { id, data }];
                            if !reply_2(payload).await {
                                error!("<Ssh: Proc {}> Stdout channel closed", id);
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
                                "<Ssh: Proc {}> Invalid data read from stdout pipe: {}",
                                id, x
                            );
                            break;
                        }
                    },
                    Ok(_) => break,
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
                    }
                    Err(_) => break,
                }
            }
        });

        // Spawn a task that sends stderr as a response
        let mut reply_2 = reply.clone();
        let stderr_task = tokio::spawn(async move {
            let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
            loop {
                match stderr.read(&mut buf) {
                    Ok(n) if n > 0 => match String::from_utf8(buf[..n].to_vec()) {
                        Ok(data) => {
                            let payload = vec![ResponseData::ProcStderr { id, data }];
                            if !reply_2(payload).await {
                                error!("<Ssh: Proc {}> Stderr channel closed", id);
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
                                "<Ssh: Proc {}> Invalid data read from stderr pipe: {}",
                                id, x
                            );
                            break;
                        }
                    },
                    Ok(_) => break,
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                        // Pause to allow buffer to fill up a little bit, avoiding
                        // spamming with a lot of smaller responses
                        tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS))
                            .await;
                    }
                    Err(_) => break,
                }
            }
        });

        let stdin_task = tokio::spawn(async move {
            while let Some(line) = stdin_rx.recv().await {
                if let Err(x) = stdin.write_all(line.as_bytes()) {
                    error!("<Ssh: Proc {}> Failed to send stdin: {}", id, x);
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
                result = child.async_wait() => {
                    match result {
                        Ok(status) => {
                            success = status.success();
                        }
                        Err(x) => {
                            error!("<Ssh: Proc {}> Waiting on process failed: {}", id, x);
                        }
                    }
                }
            }

            // Force stdin task to abort if it hasn't exited as there is no
            // point to sending any more stdin
            stdin_task.abort();

            if should_kill {
                debug!("<Ssh: Proc {}> Process killed", id);

                if let Err(x) = child.kill() {
                    error!("<Ssh: Proc {}> Unable to kill process: {}", id, x);
                }

                // NOTE: At the moment, child.kill does nothing for wezterm_ssh::SshChildProcess;
                //       so, we need to manually run kill/taskkill to make sure that the
                //       process is sent a kill signal
                if let Some(pid) = child.process_id() {
                    let _ = session.exec(&format!("kill -9 {}", pid), None).await;
                    let _ = session
                        .exec(&format!("taskkill /F /PID {}", pid), None)
                        .await;
                }
            } else {
                debug!("<Ssh: Proc {}> Process done", id);
            }

            if let Err(x) = stderr_task.await {
                error!("<Ssh: Proc {}> Join on stderr task failed: {}", id, x);
            }

            if let Err(x) = stdout_task.await {
                error!("<Ssh: Proc {}> Join on stdout task failed: {}", id, x);
            }

            state_2.lock().await.processes.remove(&id);

            let payload = vec![ResponseData::ProcDone {
                id,
                success: !should_kill && success,
                code: None,
            }];

            if !reply_2(payload).await {
                error!("<Ssh: Proc {}> Failed to send done!", id,);
            }
        });
    });

    Ok(Outgoing {
        data: ResponseData::ProcStart { id },
        post_hook: Some(post_hook),
    })
}

async fn proc_kill(
    _session: WezSession,
    state: Arc<Mutex<State>>,
    id: usize,
) -> io::Result<Outgoing> {
    if let Some(process) = state.lock().await.processes.remove(&id) {
        if process.kill_tx.send(()).await.is_ok() {
            return Ok(Outgoing::from(ResponseData::Ok));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "Unable to send kill signal to process",
    ))
}

async fn proc_stdin(
    _session: WezSession,
    state: Arc<Mutex<State>>,
    id: usize,
    data: String,
) -> io::Result<Outgoing> {
    if let Some(process) = state.lock().await.processes.get_mut(&id) {
        if process.stdin_tx.send(data).await.is_ok() {
            return Ok(Outgoing::from(ResponseData::Ok));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "Unable to send stdin to process",
    ))
}

async fn proc_list(_session: WezSession, state: Arc<Mutex<State>>) -> io::Result<Outgoing> {
    Ok(Outgoing::from(ResponseData::ProcEntries {
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
    }))
}

async fn system_info(session: WezSession) -> io::Result<Outgoing> {
    let current_dir = session
        .sftp()
        .realpath(".")
        .compat()
        .await
        .map_err(to_other_error)?;

    let first_component = current_dir.components().next();
    let is_windows =
        first_component.is_some() && matches!(first_component.unwrap(), Component::Prefix(_));
    let is_unix = current_dir.as_os_str().to_string_lossy().starts_with('/');

    let family = if is_windows {
        "windows"
    } else if is_unix {
        "unix"
    } else {
        ""
    }
    .to_string();

    Ok(Outgoing::from(ResponseData::SystemInfo {
        family,
        os: "".to_string(),
        arch: "".to_string(),
        current_dir,
        main_separator: if is_windows { '\\' } else { '/' },
    }))
}
