use crate::process::{self, SpawnResult};
use async_compat::CompatExt;
use distant_core::{
    data::{
        DirEntry, Error as DistantError, FileType, Metadata, PtySize, RunningProcess, SystemInfo,
    },
    Request, RequestData, Response, ResponseData, UnixMetadata,
};
use futures::future;
use log::*;
use std::{
    collections::HashMap,
    future::Future,
    io,
    path::{Component, PathBuf},
    pin::Pin,
    sync::Arc,
};
use tokio::sync::{mpsc, Mutex};
use wezterm_ssh::{FilePermissions, OpenFileType, OpenOptions, Session as WezSession, WriteMode};

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
    persist: bool,
    stdin_tx: mpsc::Sender<Vec<u8>>,
    kill_tx: mpsc::Sender<()>,
    resize_tx: mpsc::Sender<PtySize>,
}

type ReplyRet = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;

type PostHook = Box<dyn FnOnce(mpsc::Sender<Vec<ResponseData>>) + Send>;
struct Outgoing {
    data: ResponseData,
    post_hook: Option<PostHook>,
}

impl Outgoing {
    pub fn unsupported() -> Self {
        Self::from(ResponseData::from(io::Error::new(
            io::ErrorKind::Other,
            "Unsupported",
        )))
    }
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
    async fn inner(
        session: WezSession,
        state: Arc<Mutex<State>>,
        data: RequestData,
    ) -> io::Result<Outgoing> {
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
            RequestData::Watch { .. } => Ok(Outgoing::unsupported()),
            RequestData::Unwatch { .. } => Ok(Outgoing::unsupported()),
            RequestData::Exists { path } => exists(session, path).await,
            RequestData::Metadata {
                path,
                canonicalize,
                resolve_file_type,
            } => metadata(session, path, canonicalize, resolve_file_type).await,
            RequestData::ProcSpawn {
                cmd,
                args,
                persist,
                pty,
            } => proc_spawn(session, state, cmd, args, persist, pty).await,
            RequestData::ProcResizePty { id, size } => {
                proc_resize_pty(session, state, id, size).await
            }
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
        let session = session.clone();
        payload_tasks.push(tokio::spawn(async move {
            match inner(session, state_2, data).await {
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

    let (tx, mut rx) = mpsc::channel(1);
    tokio::spawn(async move {
        while let Some(payload) = rx.recv().await {
            if !reply(payload).await {
                break;
            }
        }
    });

    // Invoke all post hooks
    for hook in post_hooks {
        hook(tx.clone());
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
        .open_with_mode(
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
    let root_path = sftp
        .canonicalize(path)
        .compat()
        .await
        .map_err(to_other_error)?
        .into_std_path_buf();

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
            FileType::Symlink => match sftp.metadata(path.to_path_buf()).await {
                Ok(metadata) => metadata.is_dir(),
                Err(x) => {
                    errors.push(DistantError::from(to_other_error(x)));
                    continue;
                }
            },
        };

        // Determine if we continue traversing or stop
        if is_dir && (depth == 0 || next_depth <= depth) {
            match sftp
                .read_dir(path.to_path_buf())
                .compat()
                .await
                .map_err(to_other_error)
            {
                Ok(entries) => {
                    for (mut path, metadata) in entries {
                        // Canonicalize the path if specified, otherwise just return
                        // the path as is
                        path = if canonicalize {
                            match sftp.canonicalize(path).compat().await {
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
                                .map(|p| p.to_path_buf())
                                .unwrap_or(path);
                        };

                        let ft = metadata.ty;
                        to_traverse.push(DirEntry {
                            path: path.into_std_path_buf(),
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
        sftp.create_dir(path, 0o755)
            .compat()
            .await
            .map_err(to_other_error)
    }

    if all {
        // Keep trying to create a directory, moving up to parent each time a failure happens
        let mut failed_paths = Vec::new();
        let mut cur_path = path.as_path();
        let mut first_err = None;
        loop {
            match mkdir(&sftp, cur_path.to_path_buf()).await {
                Ok(_) => break,
                Err(x) => {
                    failed_paths.push(cur_path);
                    if let Some(path) = cur_path.parent() {
                        cur_path = path;

                        if first_err.is_none() {
                            first_err = Some(x);
                        }
                    } else {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            first_err.unwrap_or(x),
                        ));
                    }
                }
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
        .metadata(path.to_path_buf())
        .compat()
        .await
        .map_err(to_other_error)?;

    // If a file or symlink, we just unlink (easy)
    if stat.is_file() || stat.is_symlink() {
        sftp.remove_file(path)
            .compat()
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::PermissionDenied, x))?;
    // If directory and not forcing, we just rmdir (easy)
    } else if !force {
        sftp.remove_dir(path)
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

                for (path, stat) in sftp.read_dir(path).await.map_err(to_other_error)? {
                    to_traverse.push(DirEntry {
                        path: path.into_std_path_buf(),
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
                sftp.remove_dir(entry.path)
                    .compat()
                    .await
                    .map_err(|x| io::Error::new(io::ErrorKind::PermissionDenied, x))?;
            } else {
                sftp.remove_file(entry.path)
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
    let exists = session.sftp().symlink_metadata(path).compat().await.is_ok();

    Ok(Outgoing::from(ResponseData::Exists { value: exists }))
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
            sftp.canonicalize(path.to_path_buf())
                .compat()
                .await
                .map_err(to_other_error)?
                .into_std_path_buf(),
        )
    } else {
        None
    };

    let metadata = if resolve_file_type {
        sftp.metadata(path).compat().await.map_err(to_other_error)?
    } else {
        sftp.symlink_metadata(path)
            .compat()
            .await
            .map_err(to_other_error)?
    };

    let file_type = if metadata.is_dir() {
        FileType::Dir
    } else if metadata.is_file() {
        FileType::File
    } else {
        FileType::Symlink
    };

    Ok(Outgoing::from(ResponseData::Metadata(Metadata {
        canonicalized_path,
        file_type,
        len: metadata.size.unwrap_or(0),
        // Check that owner, group, or other has write permission (if not, then readonly)
        readonly: metadata
            .permissions
            .map(FilePermissions::is_readonly)
            .unwrap_or(true),
        accessed: metadata.accessed.map(u128::from),
        modified: metadata.modified.map(u128::from),
        created: None,
        unix: metadata.permissions.as_ref().map(|p| UnixMetadata {
            owner_read: p.owner_read,
            owner_write: p.owner_write,
            owner_exec: p.owner_exec,
            group_read: p.group_read,
            group_write: p.group_write,
            group_exec: p.group_exec,
            other_read: p.other_read,
            other_write: p.other_write,
            other_exec: p.other_exec,
        }),
        windows: None,
    })))
}

async fn proc_spawn(
    session: WezSession,
    state: Arc<Mutex<State>>,
    cmd: String,
    args: Vec<String>,
    persist: bool,
    pty: Option<PtySize>,
) -> io::Result<Outgoing> {
    let cmd_string = format!("{} {}", cmd, args.join(" "));
    debug!("<Ssh> Spawning {} (pty: {:?})", cmd_string, pty);

    let state_2 = Arc::clone(&state);
    let cleanup = |id: usize| async move {
        state_2.lock().await.processes.remove(&id);
    };

    let SpawnResult {
        id,
        stdin,
        killer,
        resizer,
        initialize,
    } = match pty {
        None => process::spawn_simple(&session, &cmd_string, cleanup).await?,
        Some(size) => process::spawn_pty(&session, &cmd_string, size, cleanup).await?,
    };

    state.lock().await.processes.insert(
        id,
        Process {
            id,
            cmd,
            args,
            persist,
            stdin_tx: stdin,
            kill_tx: killer,
            resize_tx: resizer,
        },
    );

    debug!(
        "<Ssh | Proc {}> Spawned successfully! Will enter post hook later",
        id
    );
    Ok(Outgoing {
        data: ResponseData::ProcSpawned { id },
        post_hook: Some(initialize),
    })
}

async fn proc_resize_pty(
    _session: WezSession,
    state: Arc<Mutex<State>>,
    id: usize,
    size: PtySize,
) -> io::Result<Outgoing> {
    if let Some(process) = state.lock().await.processes.get(&id) {
        if process.resize_tx.send(size).await.is_ok() {
            return Ok(Outgoing::from(ResponseData::Ok));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!("<Ssh | Proc {}> Unable to resize process", id),
    ))
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
        format!("<Ssh | Proc {}> Unable to send kill signal to process", id),
    ))
}

async fn proc_stdin(
    _session: WezSession,
    state: Arc<Mutex<State>>,
    id: usize,
    data: Vec<u8>,
) -> io::Result<Outgoing> {
    if let Some(process) = state.lock().await.processes.get_mut(&id) {
        if process.stdin_tx.send(data).await.is_ok() {
            return Ok(Outgoing::from(ResponseData::Ok));
        }
    }

    Err(io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!("<Ssh | Proc {}> Unable to send stdin to process", id),
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
                persist: p.persist,
                // TODO: Support pty size from ssh
                pty: None,
                id: p.id,
            })
            .collect(),
    }))
}

async fn system_info(session: WezSession) -> io::Result<Outgoing> {
    let current_dir = session
        .sftp()
        .canonicalize(".")
        .compat()
        .await
        .map_err(to_other_error)?
        .into_std_path_buf();

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

    Ok(Outgoing::from(ResponseData::SystemInfo(SystemInfo {
        family,
        os: "".to_string(),
        arch: "".to_string(),
        current_dir,
        main_separator: if is_windows { '\\' } else { '/' },
    })))
}
