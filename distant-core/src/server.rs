use crate::DistantApi;
use distant_net::Server;

mod process;
mod state;

use crate::{
    constants::SERVER_WATCHER_CAPACITY,
    data::{
        self, Change, ChangeKind, ChangeKindSet, DirEntry, DistantRequestData, DistantResponseData,
        FileType, Metadata, PtySize, RunningProcess, SystemInfo,
    },
    server::distant::{
        process::{Process, PtyProcess, SimpleProcess},
        state::{ProcessState, State, WatcherPath},
    },
};
use derive_more::{Display, Error, From};
use distant_net::{Request, Response};
use futures::future;
use log::*;
use notify::{Config as WatcherConfig, RecursiveMode, Watcher};
use std::{
    env,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::SystemTime,
};
use tokio::{
    io::{self, AsyncWriteExt},
    sync::{
        mpsc::{self, error::TrySendError},
        Mutex,
    },
};
use walkdir::WalkDir;

#[derive(Debug, Display, Error, From)]
pub enum ServerError {
    Io(io::Error),
    Notify(notify::Error),
    WalkDir(walkdir::Error),
}

impl From<ServerError> for DistantResponseData {
    fn from(x: ServerError) -> Self {
        match x {
            ServerError::Io(x) => Self::from(x),
            ServerError::Notify(x) => Self::from(x),
            ServerError::WalkDir(x) => Self::from(x),
        }
    }
}

async fn file_read(path: PathBuf) -> Result<Outgoing, ServerError> {
    Ok(Outgoing::from(DistantResponseData::Blob {
        data: tokio::fs::read(path).await?,
    }))
}

async fn file_read_text(path: PathBuf) -> Result<Outgoing, ServerError> {
    Ok(Outgoing::from(DistantResponseData::Text {
        data: tokio::fs::read_to_string(path).await?,
    }))
}

async fn file_write(path: PathBuf, data: impl AsRef<[u8]>) -> Result<Outgoing, ServerError> {
    tokio::fs::write(path, data).await?;
    Ok(Outgoing::from(DistantResponseData::Ok))
}

async fn file_append(path: PathBuf, data: impl AsRef<[u8]>) -> Result<Outgoing, ServerError> {
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(data.as_ref()).await?;
    Ok(Outgoing::from(DistantResponseData::Ok))
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

    Ok(Outgoing::from(DistantResponseData::DirEntries {
        entries,
        errors,
    }))
}

async fn dir_create(path: PathBuf, all: bool) -> Result<Outgoing, ServerError> {
    if all {
        tokio::fs::create_dir_all(path).await?;
    } else {
        tokio::fs::create_dir(path).await?;
    }

    Ok(Outgoing::from(DistantResponseData::Ok))
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

    Ok(Outgoing::from(DistantResponseData::Ok))
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

    Ok(Outgoing::from(DistantResponseData::Ok))
}

async fn rename(src: PathBuf, dst: PathBuf) -> Result<Outgoing, ServerError> {
    tokio::fs::rename(src, dst).await?;

    Ok(Outgoing::from(DistantResponseData::Ok))
}

async fn watch<F>(
    conn_id: usize,
    state: HState,
    reply: F,
    path: PathBuf,
    recursive: bool,
    only: Vec<ChangeKind>,
    except: Vec<ChangeKind>,
) -> Result<Outgoing, ServerError>
where
    F: FnMut(Vec<DistantResponseData>) -> ReplyRet + Clone + Send + 'static,
{
    let only = only.into_iter().collect::<ChangeKindSet>();
    let except = except.into_iter().collect::<ChangeKindSet>();
    let state_2 = Arc::clone(&state);
    let mut state = state.lock().await;

    // NOTE: Do not use get_or_insert_with since notify::recommended_watcher returns a result
    //       and we cannot unpack the result within the above function. Since we are locking
    //       our state, we can be confident that no one else is modifying the watcher option
    //       concurrently; so, we do a naive check for option being populated
    if state.watcher.is_none() {
        // NOTE: Cannot be something small like 1 as this seems to cause a deadlock sometimes
        //       with a large volume of watch requests
        let (tx, mut rx) = mpsc::channel(SERVER_WATCHER_CAPACITY);

        let mut watcher = notify::recommended_watcher(move |res| match tx.try_send(res) {
            Ok(_) => {}
            Err(TrySendError::Full(_)) => {
                warn!(
                    "Reached watcher capacity of {}! Dropping watcher event!",
                    SERVER_WATCHER_CAPACITY,
                );
            }
            Err(TrySendError::Closed(_)) => {
                warn!("Skipping watch event because watcher channel closed");
            }
        })?;

        // Attempt to configure watcher, but don't fail if these configurations fail
        match watcher.configure(WatcherConfig::PreciseEvents(true)) {
            Ok(true) => debug!("<Conn @ {}> Watcher configured for precise events", conn_id),
            Ok(false) => debug!(
                "<Conn @ {}> Watcher not configured for precise events",
                conn_id,
            ),
            Err(x) => error!(
                "<Conn @ {}> Watcher configuration for precise events failed: {}",
                conn_id, x
            ),
        }

        // Attempt to configure watcher, but don't fail if these configurations fail
        match watcher.configure(WatcherConfig::NoticeEvents(true)) {
            Ok(true) => debug!("<Conn @ {}> Watcher configured for notice events", conn_id),
            Ok(false) => debug!(
                "<Conn @ {}> Watcher not configured for notice events",
                conn_id,
            ),
            Err(x) => error!(
                "<Conn @ {}> Watcher configuration for notice events failed: {}",
                conn_id, x
            ),
        }

        let _ = state.watcher.insert(watcher);

        tokio::spawn(async move {
            while let Some(res) = rx.recv().await {
                let is_ok = match res {
                    Ok(mut x) => {
                        let mut state = state_2.lock().await;
                        let paths: Vec<_> = x.paths.drain(..).collect();
                        let kind = ChangeKind::from(x.kind);

                        trace!(
                            "<Conn @ {}> Watcher detected '{}' change for {:?}",
                            conn_id,
                            kind,
                            paths
                        );

                        fn make_res_data(
                            kind: ChangeKind,
                            paths: &[&PathBuf],
                        ) -> DistantResponseData {
                            DistantResponseData::Changed(Change {
                                kind,
                                paths: paths.iter().map(|p| p.to_path_buf()).collect(),
                            })
                        }

                        let results = state.map_paths_to_watcher_paths_and_replies(&paths);
                        let mut is_ok = true;

                        for (paths, only, reply) in results {
                            // Skip sending this change if we are not watching it
                            if (!only.is_empty() && !only.contains(&kind))
                                || (!except.is_empty() && except.contains(&kind))
                            {
                                trace!(
                                    "<Conn @ {}> Skipping change '{}' for {:?}",
                                    conn_id,
                                    kind,
                                    paths
                                );
                                continue;
                            }

                            if !reply(vec![make_res_data(kind, &paths)]).await {
                                is_ok = false;
                                break;
                            }
                        }
                        is_ok
                    }
                    Err(mut x) => {
                        let mut state = state_2.lock().await;
                        let paths: Vec<_> = x.paths.drain(..).collect();
                        let msg = x.to_string();

                        error!(
                            "<Conn @ {}> Watcher encountered an error {} for {:?}",
                            conn_id, msg, paths
                        );

                        fn make_res_data(msg: &str, paths: &[&PathBuf]) -> DistantResponseData {
                            if paths.is_empty() {
                                DistantResponseData::Error(msg.into())
                            } else {
                                DistantResponseData::Error(
                                    format!("{} about {:?}", msg, paths).into(),
                                )
                            }
                        }

                        let mut is_ok = true;

                        // If we have no paths for the errors, then we send the error to everyone
                        if paths.is_empty() {
                            trace!("<Conn @ {}> Relaying error to all watchers", conn_id);
                            for reply in state.watcher_paths.values_mut() {
                                if !reply(vec![make_res_data(&msg, &[])]).await {
                                    is_ok = false;
                                    break;
                                }
                            }
                        // Otherwise, figure out the relevant watchers from our paths and
                        // send the error to them
                        } else {
                            let results = state.map_paths_to_watcher_paths_and_replies(&paths);

                            trace!(
                                "<Conn @ {}> Relaying error to {} watchers",
                                conn_id,
                                results.len()
                            );
                            for (paths, _, reply) in results {
                                if !reply(vec![make_res_data(&msg, &paths)]).await {
                                    is_ok = false;
                                    break;
                                }
                            }
                        }

                        is_ok
                    }
                };

                if !is_ok {
                    error!("<Conn @ {}> Watcher channel closed", conn_id);
                    break;
                }
            }
        });
    }

    match state.watcher.as_mut() {
        Some(watcher) => {
            let wp = WatcherPath::new(&path, recursive, only)?;
            watcher.watch(
                path.as_path(),
                if recursive {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                },
            )?;
            debug!("<Conn @ {}> Now watching {:?}", conn_id, wp.path());
            state.watcher_paths.insert(wp, Box::new(reply));
            Ok(Outgoing::from(DistantResponseData::Ok))
        }
        None => Err(ServerError::Io(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("<Conn @ {}> Unable to initialize watcher", conn_id),
        ))),
    }
}

async fn unwatch(conn_id: usize, state: HState, path: PathBuf) -> Result<Outgoing, ServerError> {
    if let Some(watcher) = state.lock().await.watcher.as_mut() {
        watcher.unwatch(path.as_path())?;
        // TODO: This also needs to remove any path that matches in either raw form
        //       or canonicalized form from the map of PathBuf -> ReplyFn
        return Ok(Outgoing::from(DistantResponseData::Ok));
    }

    Err(ServerError::Io(io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!(
            "<Conn @ {}> Unable to unwatch as watcher not initialized",
            conn_id,
        ),
    )))
}

async fn exists(path: PathBuf) -> Result<Outgoing, ServerError> {
    // Following experimental `std::fs::try_exists`, which checks the error kind of the
    // metadata lookup to see if it is not found and filters accordingly
    Ok(match tokio::fs::metadata(path.as_path()).await {
        Ok(_) => Outgoing::from(DistantResponseData::Exists { value: true }),
        Err(x) if x.kind() == io::ErrorKind::NotFound => {
            Outgoing::from(DistantResponseData::Exists { value: false })
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

    Ok(Outgoing::from(DistantResponseData::Metadata(Metadata {
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

        #[cfg(unix)]
        unix: Some({
            use std::os::unix::prelude::*;
            let mode = metadata.mode();
            crate::data::UnixMetadata::from(mode)
        }),
        #[cfg(not(unix))]
        unix: None,

        #[cfg(windows)]
        windows: Some({
            use std::os::windows::prelude::*;
            let attributes = metadata.file_attributes();
            crate::data::WindowsMetadata::from(attributes)
        }),
        #[cfg(not(windows))]
        windows: None,
    })))
}

async fn proc_spawn<F>(
    conn_id: usize,
    state: HState,
    reply: F,
    cmd: String,
    persist: bool,
    pty: Option<PtySize>,
) -> Result<Outgoing, ServerError>
where
    F: FnMut(Vec<DistantResponseData>) -> ReplyRet + Clone + Send + 'static,
{
    debug!("<Conn @ {}> Spawning {}", conn_id, cmd);

    // Build out the command and args from our string
    let (cmd, args) = match cmd.split_once(" ") {
        Some((cmd_str, args_str)) => (
            cmd.to_string(),
            args_str.split(" ").map(ToString::to_string).collect(),
        ),
        None => (cmd, Vec::new()),
    };

    let mut child: Box<dyn Process> = match pty {
        Some(size) => Box::new(PtyProcess::spawn(cmd.clone(), args.clone(), size)?),
        None => Box::new(SimpleProcess::spawn(cmd.clone(), args.clone())?),
    };

    let id = child.id();
    let stdin = child.take_stdin();
    let stdout = child.take_stdout();
    let stderr = child.take_stderr();
    let killer = child.clone_killer();
    let pty = child.clone_pty();

    let state_2 = Arc::clone(&state);
    let post_hook = Box::new(move || {
        // Spawn a task that sends stdout as a response
        if let Some(mut stdout) = stdout {
            let mut reply_2 = reply.clone();
            let _ = tokio::spawn(async move {
                loop {
                    match stdout.recv().await {
                        Ok(Some(data)) => {
                            let payload = vec![DistantResponseData::ProcStdout { id, data }];
                            if !reply_2(payload).await {
                                error!("<Conn @ {} | Proc {}> Stdout channel closed", conn_id, id);
                                break;
                            }
                        }
                        Ok(None) => break,
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
        }

        // Spawn a task that sends stderr as a response
        if let Some(mut stderr) = stderr {
            let mut reply_2 = reply.clone();
            let _ = tokio::spawn(async move {
                loop {
                    match stderr.recv().await {
                        Ok(Some(data)) => {
                            let payload = vec![DistantResponseData::ProcStderr { id, data }];
                            if !reply_2(payload).await {
                                error!("<Conn @ {} | Proc {}> Stderr channel closed", conn_id, id);
                                break;
                            }
                        }
                        Ok(None) => break,
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
        }

        // Spawn a task that waits on the process to exit but can also
        // kill the process when triggered
        let mut reply_2 = reply.clone();
        let _ = tokio::spawn(async move {
            let status = child.wait().await;
            debug!("<Conn @ {} | Proc {}> Completed {:?}", conn_id, id, status);

            state_2.lock().await.remove_process(conn_id, id);

            match status {
                Ok(status) => {
                    let payload = vec![DistantResponseData::ProcDone {
                        id,
                        success: status.success,
                        code: status.code,
                    }];
                    if !reply_2(payload).await {
                        error!("<Conn @ {} | Proc {}> Failed to send done", conn_id, id,);
                    }
                }
                Err(x) => {
                    let payload = vec![DistantResponseData::from(x)];
                    if !reply_2(payload).await {
                        error!(
                            "<Conn @ {} | Proc {}> Failed to send error for waiting",
                            conn_id, id,
                        );
                    }
                }
            }
        });
    });

    state.lock().await.push_process_state(
        conn_id,
        ProcessState {
            cmd,
            args,
            persist,
            id,
            stdin,
            killer,
            pty,
        },
    );

    debug!(
        "<Conn @ {} | Proc {}> Spawned successfully! Will enter post hook later",
        conn_id, id
    );
    Ok(Outgoing {
        data: DistantResponseData::ProcSpawned { id },
        post_hook: Some(post_hook),
    })
}

async fn proc_kill(conn_id: usize, state: HState, id: usize) -> Result<Outgoing, ServerError> {
    if let Some(mut process) = state.lock().await.processes.remove(&id) {
        if process.killer.kill().await.is_ok() {
            return Ok(Outgoing::from(DistantResponseData::Ok));
        }
    }

    Err(ServerError::Io(io::Error::new(
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
    data: Vec<u8>,
) -> Result<Outgoing, ServerError> {
    if let Some(process) = state.lock().await.processes.get_mut(&id) {
        if let Some(stdin) = process.stdin.as_mut() {
            if stdin.send(&data).await.is_ok() {
                return Ok(Outgoing::from(DistantResponseData::Ok));
            }
        }
    }

    Err(ServerError::Io(io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!(
            "<Conn @ {} | Proc {}> Unable to send stdin to process",
            conn_id, id,
        ),
    )))
}

async fn proc_resize_pty(
    conn_id: usize,
    state: HState,
    id: usize,
    size: PtySize,
) -> Result<Outgoing, ServerError> {
    if let Some(process) = state.lock().await.processes.get(&id) {
        let _ = process.pty.resize_pty(size)?;

        return Ok(Outgoing::from(DistantResponseData::Ok));
    }

    Err(ServerError::Io(io::Error::new(
        io::ErrorKind::BrokenPipe,
        format!(
            "<Conn @ {} | Proc {}> Unable to resize pty to {:?}",
            conn_id, id, size,
        ),
    )))
}

async fn proc_list(state: HState) -> Result<Outgoing, ServerError> {
    Ok(Outgoing::from(DistantResponseData::ProcEntries {
        entries: state
            .lock()
            .await
            .processes
            .values()
            .map(|p| RunningProcess {
                cmd: p.cmd.to_string(),
                args: p.args.clone(),
                persist: p.persist,
                // TODO: Support retrieving current pty size
                pty: None,
                id: p.id,
            })
            .collect(),
    }))
}

async fn system_info() -> Result<Outgoing, ServerError> {
    Ok(Outgoing::from(DistantResponseData::SystemInfo(
        SystemInfo {
            family: env::consts::FAMILY.to_string(),
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
            current_dir: env::current_dir().unwrap_or_default(),
            main_separator: std::path::MAIN_SEPARATOR,
        },
    )))
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

        let req = Request::new("test-tenant", vec![DistantRequestData::FileRead { path }]);

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::FileRead {
                path: file.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            DistantResponseData::Blob { data } => assert_eq!(data, b"some file contents"),
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn file_read_text_should_send_error_if_fails_to_read_file() {
        let (conn_id, state, tx, mut rx) = setup(1);

        let temp = assert_fs::TempDir::new().unwrap();
        let path = temp.child("missing-file").path().to_path_buf();

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::FileReadText { path }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::FileReadText {
                path: file.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            DistantResponseData::Text { data } => assert_eq!(data, "some file contents"),
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
            vec![DistantRequestData::FileWrite {
                path: file.path().to_path_buf(),
                data: b"some text".to_vec(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::FileWrite {
                path: file.path().to_path_buf(),
                data: b"some text".to_vec(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::FileWriteText {
                path: file.path().to_path_buf(),
                text: String::from("some text"),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::FileWriteText {
                path: file.path().to_path_buf(),
                text: String::from("some text"),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::FileAppend {
                path: file.path().to_path_buf(),
                data: b"some extra contents".to_vec(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::FileAppend {
                path: file.path().to_path_buf(),
                data: b"some extra contents".to_vec(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::FileAppend {
                path: file.path().to_path_buf(),
                data: b"some extra contents".to_vec(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::FileAppendText {
                path: file.path().to_path_buf(),
                text: String::from("some extra contents"),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::FileAppendText {
                path: file.path().to_path_buf(),
                text: "some extra contents".to_string(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::FileAppendText {
                path: file.path().to_path_buf(),
                text: String::from("some extra contents"),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::DirRead {
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
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::DirRead {
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
            DistantResponseData::DirEntries { entries, .. } => {
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
            vec![DistantRequestData::DirRead {
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
            DistantResponseData::DirEntries { entries, .. } => {
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
            vec![DistantRequestData::DirRead {
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
            DistantResponseData::DirEntries { entries, .. } => {
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
            vec![DistantRequestData::DirRead {
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
            DistantResponseData::DirEntries { entries, .. } => {
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
            vec![DistantRequestData::DirRead {
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
            DistantResponseData::DirEntries { entries, .. } => {
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
            vec![DistantRequestData::DirCreate {
                path: path.to_path_buf(),
                all: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::DirCreate {
                path: path.to_path_buf(),
                all: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::DirCreate {
                path: path.to_path_buf(),
                all: true,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::Remove {
                path: file.path().to_path_buf(),
                force: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::Remove {
                path: dir.path().to_path_buf(),
                force: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::Remove {
                path: dir.path().to_path_buf(),
                force: true,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::Remove {
                path: file.path().to_path_buf(),
                force: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::Copy {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::Rename {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::Rename {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
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
            vec![DistantRequestData::Rename {
                src: src.path().to_path_buf(),
                dst: dst.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Verify that we moved the file
        src.assert(predicate::path::missing());
        dst.assert("some text");
    }

    /// Validates a response as being a series of changes that include the provided paths
    fn validate_changed_paths(
        res: &Response,
        expected_paths: &[PathBuf],
        should_panic: bool,
    ) -> bool {
        match &res.payload[0] {
            DistantResponseData::Changed(change) if should_panic => {
                let paths: Vec<PathBuf> = change
                    .paths
                    .iter()
                    .map(|x| x.canonicalize().unwrap())
                    .collect();
                assert_eq!(paths, expected_paths, "Wrong paths reported: {:?}", change);

                true
            }
            DistantResponseData::Changed(change) => {
                let paths: Vec<PathBuf> = change
                    .paths
                    .iter()
                    .map(|x| x.canonicalize().unwrap())
                    .collect();
                paths == expected_paths
            }
            x if should_panic => panic!("Unexpected response: {:?}", x),
            _ => false,
        }
    }

    #[tokio::test]
    async fn watch_should_support_watching_a_single_file() {
        // NOTE: Supporting multiple replies being sent back as part of creating, modifying, etc.
        let (conn_id, state, tx, mut rx) = setup(100);
        let temp = assert_fs::TempDir::new().unwrap();

        let file = temp.child("file");
        file.touch().unwrap();

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::Watch {
                path: file.path().to_path_buf(),
                recursive: false,
                only: Default::default(),
                except: Default::default(),
            }],
        );

        // NOTE: We need to clone state so we don't drop the watcher
        //       as part of dropping the state
        process(conn_id, Arc::clone(&state), req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Update the file and verify we get a notification
        file.write_str("some text").unwrap();

        let res = rx
            .recv()
            .await
            .expect("Channel closed before we got change");
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        validate_changed_paths(
            &res,
            &[file.path().to_path_buf().canonicalize().unwrap()],
            /* should_panic */ true,
        );
    }

    #[tokio::test]
    async fn watch_should_support_watching_a_directory_recursively() {
        // NOTE: Supporting multiple replies being sent back as part of creating, modifying, etc.
        let (conn_id, state, tx, mut rx) = setup(100);
        let temp = assert_fs::TempDir::new().unwrap();

        let file = temp.child("file");
        file.touch().unwrap();

        let dir = temp.child("dir");
        dir.create_dir_all().unwrap();

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::Watch {
                path: temp.path().to_path_buf(),
                recursive: true,
                only: Default::default(),
                except: Default::default(),
            }],
        );

        // NOTE: We need to clone state so we don't drop the watcher
        //       as part of dropping the state
        process(conn_id, Arc::clone(&state), req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Ok),
            "Unexpected response: {:?}",
            res.payload[0]
        );

        // Update the file and verify we get a notification
        file.write_str("some text").unwrap();

        // Create a nested file and verify we get a notification
        let nested_file = dir.child("nested-file");
        nested_file.write_str("some text").unwrap();

        // Sleep a bit to give time to get all changes happening
        // TODO: Can we slim down this sleep? Or redesign test in some other way?
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Collect all responses, as we may get multiple for interactions within a directory
        let mut responses = Vec::new();
        while let Ok(res) = rx.try_recv() {
            responses.push(res);
        }

        // Validate that we have at least one change reported for each of our paths
        assert!(
            responses.len() >= 2,
            "Less than expected total responses: {:?}",
            responses
        );

        let path = file.path().to_path_buf();
        assert!(
            responses.iter().any(|res| validate_changed_paths(
                res,
                &[file.path().to_path_buf().canonicalize().unwrap()],
                /* should_panic */ false,
            )),
            "Missing {:?} in {:?}",
            path,
            responses
                .iter()
                .map(|x| format!("{:?}", x))
                .collect::<Vec<String>>(),
        );

        let path = nested_file.path().to_path_buf();
        assert!(
            responses.iter().any(|res| validate_changed_paths(
                res,
                &[file.path().to_path_buf().canonicalize().unwrap()],
                /* should_panic */ false,
            )),
            "Missing {:?} in {:?}",
            path,
            responses
                .iter()
                .map(|x| format!("{:?}", x))
                .collect::<Vec<String>>(),
        );
    }

    #[tokio::test]
    async fn watch_should_report_changes_using_the_request_id() {
        // NOTE: Supporting multiple replies being sent back as part of creating, modifying, etc.
        let (conn_id, state, tx, mut rx) = setup(100);
        let temp = assert_fs::TempDir::new().unwrap();

        let file_1 = temp.child("file_1");
        file_1.touch().unwrap();

        let file_2 = temp.child("file_2");
        file_2.touch().unwrap();

        // Sleep a bit to give time to get all changes happening
        // TODO: Can we slim down this sleep? Or redesign test in some other way?
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Initialize watch on file 1
        let file_1_origin_id = {
            let req = Request::new(
                "test-tenant",
                vec![DistantRequestData::Watch {
                    path: file_1.path().to_path_buf(),
                    recursive: false,
                    only: Default::default(),
                    except: Default::default(),
                }],
            );
            let origin_id = req.id;

            // NOTE: We need to clone state so we don't drop the watcher
            //       as part of dropping the state
            process(conn_id, Arc::clone(&state), req, tx.clone())
                .await
                .unwrap();

            let res = rx.recv().await.unwrap();
            assert_eq!(res.payload.len(), 1, "Wrong payload size");
            assert!(
                matches!(res.payload[0], DistantResponseData::Ok),
                "Unexpected response: {:?}",
                res.payload[0]
            );

            origin_id
        };

        // Initialize watch on file 2
        let file_2_origin_id = {
            let req = Request::new(
                "test-tenant",
                vec![DistantRequestData::Watch {
                    path: file_2.path().to_path_buf(),
                    recursive: false,
                    only: Default::default(),
                    except: Default::default(),
                }],
            );
            let origin_id = req.id;

            // NOTE: We need to clone state so we don't drop the watcher
            //       as part of dropping the state
            process(conn_id, Arc::clone(&state), req, tx).await.unwrap();

            let res = rx.recv().await.unwrap();
            assert_eq!(res.payload.len(), 1, "Wrong payload size");
            assert!(
                matches!(res.payload[0], DistantResponseData::Ok),
                "Unexpected response: {:?}",
                res.payload[0]
            );

            origin_id
        };

        // Update the files and verify we get notifications from different origins
        {
            file_1.write_str("some text").unwrap();
            let res = rx
                .recv()
                .await
                .expect("Channel closed before we got change");
            assert_eq!(res.payload.len(), 1, "Wrong payload size");
            validate_changed_paths(
                &res,
                &[file_1.path().to_path_buf().canonicalize().unwrap()],
                /* should_panic */ true,
            );
            assert_eq!(res.origin_id, file_1_origin_id, "Wrong origin id (file 1)");

            // Process any extra messages (we might get create, content, and more)
            loop {
                // Sleep a bit to give time to get all changes happening
                // TODO: Can we slim down this sleep? Or redesign test in some other way?
                tokio::time::sleep(Duration::from_millis(100)).await;

                if rx.try_recv().is_err() {
                    break;
                }
            }
        }

        // Update the files and verify we get notifications from different origins
        {
            file_2.write_str("some text").unwrap();
            let res = rx
                .recv()
                .await
                .expect("Channel closed before we got change");
            assert_eq!(res.payload.len(), 1, "Wrong payload size");
            validate_changed_paths(
                &res,
                &[file_2.path().to_path_buf().canonicalize().unwrap()],
                /* should_panic */ true,
            );
            assert_eq!(res.origin_id, file_2_origin_id, "Wrong origin id (file 2)");
        }
    }

    #[tokio::test]
    async fn exists_should_send_true_if_path_exists() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.touch().unwrap();

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::Exists {
                path: file.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert_eq!(res.payload[0], DistantResponseData::Exists { value: true });
    }

    #[tokio::test]
    async fn exists_should_send_false_if_path_does_not_exist() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::Exists {
                path: file.path().to_path_buf(),
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert_eq!(res.payload[0], DistantResponseData::Exists { value: false });
    }

    #[tokio::test]
    async fn metadata_should_send_error_on_failure() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::Metadata {
                path: file.path().to_path_buf(),
                canonicalize: false,
                resolve_file_type: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::Metadata {
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
                DistantResponseData::Metadata(Metadata {
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

    #[cfg(unix)]
    #[tokio::test]
    async fn metadata_should_include_unix_specific_metadata_on_unix_platform() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::Metadata {
                path: file.path().to_path_buf(),
                canonicalize: false,
                resolve_file_type: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        match &res.payload[0] {
            DistantResponseData::Metadata(Metadata { unix, windows, .. }) => {
                assert!(unix.is_some(), "Unexpectedly missing unix metadata on unix");
                assert!(
                    windows.is_none(),
                    "Unexpectedly got windows metadata on unix"
                );
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn metadata_should_include_unix_specific_metadata_on_windows_platform() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("file");
        file.write_str("some text").unwrap();

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::Metadata {
                path: file.path().to_path_buf(),
                canonicalize: false,
                resolve_file_type: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        match &res.payload[0] {
            DistantResponseData::Metadata(Metadata { unix, windows, .. }) => {
                assert!(
                    windows.is_some(),
                    "Unexpectedly missing windows metadata on windows"
                );
                assert!(unix.is_none(), "Unexpectedly got unix metadata on windows");
            }
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn metadata_should_send_back_metadata_on_dir_if_exists() {
        let (conn_id, state, tx, mut rx) = setup(1);
        let temp = assert_fs::TempDir::new().unwrap();
        let dir = temp.child("dir");
        dir.create_dir_all().unwrap();

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::Metadata {
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
                DistantResponseData::Metadata(Metadata {
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
            vec![DistantRequestData::Metadata {
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
                DistantResponseData::Metadata(Metadata {
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
            vec![DistantRequestData::Metadata {
                path: symlink.path().to_path_buf(),
                canonicalize: true,
                resolve_file_type: false,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            DistantResponseData::Metadata(Metadata {
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
            vec![DistantRequestData::Metadata {
                path: symlink.path().to_path_buf(),
                canonicalize: false,
                resolve_file_type: true,
            }],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        match &res.payload[0] {
            DistantResponseData::Metadata(Metadata {
                file_type: FileType::File,
                ..
            }) => {}
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn proc_spawn_should_send_error_on_failure() {
        let (conn_id, state, tx, mut rx) = setup(1);

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::ProcSpawn {
                cmd: DOES_NOT_EXIST_BIN.to_str().unwrap().to_string(),
                persist: false,
                pty: None,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(&res.payload[0], DistantResponseData::Error(_)),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn proc_spawn_should_send_back_proc_start_on_success() {
        let (conn_id, state, tx, mut rx) = setup(1);

        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::ProcSpawn {
                cmd: format!("{} {}", SCRIPT_RUNNER, ECHO_ARGS_TO_STDOUT_SH),
                persist: false,
                pty: None,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(&res.payload[0], DistantResponseData::ProcSpawned { .. }),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    // NOTE: Ignoring on windows because it's using WSL which wants a Linux path
    //       with / but thinks it's on windows and is providing \
    #[tokio::test]
    #[cfg_attr(windows, ignore)]
    async fn proc_spawn_should_send_back_stdout_periodically_when_available() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Run a program that echoes to stdout
        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::ProcSpawn {
                cmd: format!("{} {} some stdout", SCRIPT_RUNNER, ECHO_ARGS_TO_STDOUT_SH),
                persist: false,
                pty: None,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(&res.payload[0], DistantResponseData::ProcSpawned { .. }),
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
                DistantResponseData::ProcStdout { data, .. } => {
                    assert_eq!(data, b"some stdout", "Got wrong stdout");
                    got_stdout = true;
                }
                DistantResponseData::ProcDone { success, .. } => {
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
    async fn proc_spawn_should_send_back_stderr_periodically_when_available() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Run a program that echoes to stderr
        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::ProcSpawn {
                cmd: format!("{} {} some stderr", SCRIPT_RUNNER, ECHO_ARGS_TO_STDERR_SH),
                persist: false,
                pty: None,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert!(
            matches!(&res.payload[0], DistantResponseData::ProcSpawned { .. }),
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
                DistantResponseData::ProcStderr { data, .. } => {
                    assert_eq!(data, b"some stderr", "Got wrong stderr");
                    got_stderr = true;
                }
                DistantResponseData::ProcDone { success, .. } => {
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
    async fn proc_spawn_should_clear_process_from_state_when_done() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Run a program that ends after a little bit
        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::ProcSpawn {
                cmd: format!("{} {} 0.1", SCRIPT_RUNNER, SLEEP_SH),
                persist: false,
                pty: None,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        let id = match &res.payload[0] {
            DistantResponseData::ProcSpawned { id } => *id,
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
    async fn proc_spawn_should_clear_process_from_state_when_killed() {
        let (conn_id, state, tx, mut rx) = setup(1);

        // Run a program that ends slowly
        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::ProcSpawn {
                cmd: format!("{} {} 1", SCRIPT_RUNNER, SLEEP_SH),
                persist: false,
                pty: None,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        let id = match &res.payload[0] {
            DistantResponseData::ProcSpawned { id } => *id,
            x => panic!("Unexpected response: {:?}", x),
        };

        // Verify that the state has the process
        assert!(
            state.lock().await.processes.contains_key(&id),
            "Process {} not in state",
            id
        );

        // Send kill signal
        let req = Request::new("test-tenant", vec![DistantRequestData::ProcKill { id }]);
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
            vec![DistantRequestData::ProcKill { id: 0xDEADBEEF }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        // Verify that we get an error
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::ProcSpawn {
                cmd: format!("{} {} 1", SCRIPT_RUNNER, SLEEP_SH),
                persist: false,
                pty: None,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        // Second, grab the id of the started process
        let id = match &res.payload[0] {
            DistantResponseData::ProcSpawned { id } => *id,
            x => panic!("Unexpected response: {:?}", x),
        };

        // Third, send kill for process
        let req = Request::new("test-tenant", vec![DistantRequestData::ProcKill { id }]);

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
                DistantResponseData::Ok => got_ok = true,
                DistantResponseData::ProcDone { success, .. } => {
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
            vec![DistantRequestData::ProcStdin {
                id: 0xDEADBEEF,
                data: b"some input".to_vec(),
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        // Verify that we get an error
        assert!(
            matches!(res.payload[0], DistantResponseData::Error(_)),
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
            vec![DistantRequestData::ProcSpawn {
                cmd: format!("{} {}", SCRIPT_RUNNER, ECHO_STDIN_TO_STDOUT_SH),
                persist: false,
                pty: None,
            }],
        );

        process(conn_id, Arc::clone(&state), req, tx.clone())
            .await
            .unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");

        // Second, grab the id of the started process
        let id = match &res.payload[0] {
            DistantResponseData::ProcSpawned { id } => *id,
            x => panic!("Unexpected response: {:?}", x),
        };

        // Third, send stdin to the remote process
        let req = Request::new(
            "test-tenant",
            vec![DistantRequestData::ProcStdin {
                id,
                data: b"hello world\n".to_vec(),
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
                DistantResponseData::Ok => got_ok = true,
                DistantResponseData::ProcStdout { data, .. } => {
                    assert_eq!(data, b"hello world\n", "Mirrored data didn't match");
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
                DistantRequestData::ProcSpawn {
                    cmd: format!("{} {} 1", SCRIPT_RUNNER, SLEEP_SH),
                    persist: false,
                    pty: None,
                },
                DistantRequestData::ProcList {},
            ],
        );

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 2, "Wrong payload size");

        // Grab the id of the started process
        let id = match &res.payload[0] {
            DistantResponseData::ProcSpawned { id } => *id,
            x => panic!("Unexpected response: {:?}", x),
        };

        // Verify our process shows up in our entry list
        assert_eq!(
            res.payload[1],
            DistantResponseData::ProcEntries {
                entries: vec![RunningProcess {
                    cmd: SCRIPT_RUNNER.to_string(),
                    args: vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")],
                    persist: false,
                    pty: None,
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

        let req = Request::new("test-tenant", vec![DistantRequestData::SystemInfo {}]);

        process(conn_id, state, req, tx).await.unwrap();

        let res = rx.recv().await.unwrap();
        assert_eq!(res.payload.len(), 1, "Wrong payload size");
        assert_eq!(
            res.payload[0],
            DistantResponseData::SystemInfo(SystemInfo {
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
