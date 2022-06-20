use crate::process::{spawn_pty, spawn_simple, SpawnResult};
use async_compat::CompatExt;
use async_trait::async_trait;
use distant_core::{
    data::{DirEntry, FileType, Metadata, PtySize, SystemInfo, UnixMetadata},
    DistantApi, DistantCtx,
};
use log::*;
use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Component, PathBuf},
    sync::{Arc, Weak},
};
use tokio::sync::{mpsc, RwLock};
use wezterm_ssh::{FilePermissions, OpenFileType, OpenOptions, Session as WezSession, WriteMode};

fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::Other, err)
}

#[derive(Default)]
pub struct ConnectionState {
    /// List of process ids that will be killed when the connection terminates
    processes: Arc<RwLock<HashSet<usize>>>,

    /// Internal reference to global process list for removals
    /// NOTE: Initialized during `on_connection` of [`DistantApi`]
    global_processes: Weak<RwLock<HashMap<usize, Process>>>,
}

struct Process {
    stdin_tx: mpsc::Sender<Vec<u8>>,
    kill_tx: mpsc::Sender<()>,
    resize_tx: mpsc::Sender<PtySize>,
}

/// Represents implementation of [`DistantApi`] for SSH
pub struct SshDistantApi {
    /// Internal ssh session
    session: WezSession,

    /// Global tracking of running processes by id
    processes: Arc<RwLock<HashMap<usize, Process>>>,
}

impl SshDistantApi {
    pub fn new(session: WezSession) -> Self {
        Self {
            session,
            processes: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl DistantApi for SshDistantApi {
    type LocalData = ConnectionState;

    async fn on_connection(&self, mut local_data: Self::LocalData) -> Self::LocalData {
        local_data.global_processes = Arc::downgrade(&self.processes);
        local_data
    }

    async fn read_file(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
    ) -> io::Result<Vec<u8>> {
        debug!(
            "[Conn {}] Reading bytes from file {:?}",
            ctx.connection_id, path
        );

        use smol::io::AsyncReadExt;
        let mut file = self
            .session
            .sftp()
            .open(path)
            .compat()
            .await
            .map_err(to_other_error)?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).compat().await?;
        Ok(contents.into_bytes())
    }

    async fn read_file_text(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
    ) -> io::Result<String> {
        debug!(
            "[Conn {}] Reading text from file {:?}",
            ctx.connection_id, path
        );

        use smol::io::AsyncReadExt;
        let mut file = self
            .session
            .sftp()
            .open(path)
            .compat()
            .await
            .map_err(to_other_error)?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).compat().await?;
        Ok(contents)
    }

    async fn write_file(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: Vec<u8>,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Writing bytes to file {:?}",
            ctx.connection_id, path
        );

        use smol::io::AsyncWriteExt;
        let mut file = self
            .session
            .sftp()
            .create(path)
            .compat()
            .await
            .map_err(to_other_error)?;

        file.write_all(data.as_ref()).compat().await?;

        Ok(())
    }

    async fn write_file_text(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: String,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Writing text to file {:?}",
            ctx.connection_id, path
        );

        use smol::io::AsyncWriteExt;
        let mut file = self
            .session
            .sftp()
            .create(path)
            .compat()
            .await
            .map_err(to_other_error)?;

        file.write_all(data.as_ref()).compat().await?;

        Ok(())
    }

    async fn append_file(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: Vec<u8>,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Appending bytes to file {:?}",
            ctx.connection_id, path
        );

        use smol::io::AsyncWriteExt;
        let mut file = self
            .session
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
        Ok(())
    }

    async fn append_file_text(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        data: String,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Appending text to file {:?}",
            ctx.connection_id, path
        );

        use smol::io::AsyncWriteExt;
        let mut file = self
            .session
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
        Ok(())
    }

    async fn read_dir(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> io::Result<(Vec<DirEntry>, Vec<io::Error>)> {
        debug!(
            "[Conn {}] Reading directory {:?} {{depth: {}, absolute: {}, canonicalize: {}, include_root: {}}}",
            ctx.connection_id, path, depth, absolute, canonicalize, include_root
        );

        let sftp = self.session.sftp();

        // Canonicalize our provided path to ensure that it is exists, not a loop, and absolute
        let root_path = sftp
            .canonicalize(path)
            .compat()
            .await
            .map_err(to_other_error)?
            .into_std_path_buf();

        // Build up our entry list
        let mut entries = Vec::new();
        let mut errors: Vec<io::Error> = Vec::new();

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
                        errors.push(to_other_error(x));
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
                                        errors.push(to_other_error(x));
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
                    Err(x) => errors.push(x),
                }
            }
        }

        // Sort entries by filename
        entries.sort_unstable_by_key(|e| e.path.to_path_buf());

        Ok((entries, errors))
    }

    async fn create_dir(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        all: bool,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Creating directory {:?} {{all: {}}}",
            ctx.connection_id, path, all
        );

        let sftp = self.session.sftp();

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

        Ok(())
    }

    async fn remove(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        force: bool,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Removing {:?} {{force: {}}}",
            ctx.connection_id, path, force
        );

        let sftp = self.session.sftp();

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

        Ok(())
    }

    async fn copy(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        src: PathBuf,
        dst: PathBuf,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Copying {:?} to {:?}",
            ctx.connection_id, src, dst
        );

        // NOTE: SFTP does not provide a remote-to-remote copy method, so we instead execute
        //       a program and hope that it applies, starting with the Unix/BSD/GNU cp method
        //       and switch to Window's xcopy if the former fails

        // Unix cp -R <src> <dst>
        let unix_result = self
            .session
            .exec(&format!("cp -R {:?} {:?}", src, dst), None)
            .compat()
            .await;

        let failed = unix_result.is_err() || {
            let exit_status = unix_result.unwrap().child.async_wait().compat().await;
            exit_status.is_err() || !exit_status.unwrap().success()
        };

        // Windows xcopy <src> <dst> /s /e
        if failed {
            let exit_status = self
                .session
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

        Ok(())
    }

    async fn rename(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        src: PathBuf,
        dst: PathBuf,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Renaming {:?} to {:?}",
            ctx.connection_id, src, dst
        );

        self.session
            .sftp()
            .rename(src, dst, Default::default())
            .compat()
            .await
            .map_err(to_other_error)?;

        Ok(())
    }

    async fn exists(&self, ctx: DistantCtx<Self::LocalData>, path: PathBuf) -> io::Result<bool> {
        debug!("[Conn {}] Checking if {:?} exists", ctx.connection_id, path);

        // NOTE: SFTP does not provide a means to check if a path exists that can be performed
        // separately from getting permission errors; so, we just assume any error means that the path
        // does not exist
        let exists = self
            .session
            .sftp()
            .symlink_metadata(path)
            .compat()
            .await
            .is_ok();
        Ok(exists)
    }

    async fn metadata(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        path: PathBuf,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> io::Result<Metadata> {
        debug!(
            "[Conn {}] Reading metadata for {:?} {{canonicalize: {}, resolve_file_type: {}}}",
            ctx.connection_id, path, canonicalize, resolve_file_type
        );

        let sftp = self.session.sftp();
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

        Ok(Metadata {
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
        })
    }

    async fn proc_spawn(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        cmd: String,
        persist: bool,
        pty: Option<PtySize>,
    ) -> io::Result<usize> {
        debug!(
            "[Conn {}] Spawning {} {{persist: {}, pty: {:?}}}",
            ctx.connection_id, cmd, persist, pty
        );
        debug!("[Ssh] Spawning {} (pty: {:?})", cmd, pty);

        let global_processes = Arc::downgrade(&self.processes);
        let local_processes = Arc::downgrade(&ctx.local_data.processes);
        let cleanup = |id: usize| async move {
            if let Some(processes) = Weak::upgrade(&global_processes) {
                processes.write().await.remove(&id);
            }
            if let Some(processes) = Weak::upgrade(&local_processes) {
                processes.write().await.remove(&id);
            }
        };

        let SpawnResult {
            id,
            stdin,
            killer,
            resizer,
        } = match pty {
            None => spawn_simple(&self.session, &cmd, ctx.reply.clone_reply(), cleanup).await?,
            Some(size) => {
                spawn_pty(&self.session, &cmd, size, ctx.reply.clone_reply(), cleanup).await?
            }
        };

        // If the process will be killed when the connection ends, we want to add it
        // to our local data
        if !persist {
            ctx.local_data.processes.write().await.insert(id);
        }

        self.processes.write().await.insert(
            id,
            Process {
                stdin_tx: stdin,
                kill_tx: killer,
                resize_tx: resizer,
            },
        );

        debug!(
            "[Ssh | Proc {}] Spawned successfully! Will enter post hook later",
            id
        );
        Ok(id)
    }

    async fn proc_kill(&self, ctx: DistantCtx<Self::LocalData>, id: usize) -> io::Result<()> {
        debug!("[Conn {}] Killing process {}", ctx.connection_id, id);

        if let Some(process) = self.processes.read().await.get(&id) {
            if process.kill_tx.send(()).await.is_ok() {
                return Ok(());
            }
        }

        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("[Ssh | Proc {}] Unable to send kill signal to process", id),
        ))
    }

    async fn proc_stdin(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        id: usize,
        data: Vec<u8>,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Sending stdin to process {}",
            ctx.connection_id, id
        );

        if let Some(process) = self.processes.read().await.get(&id) {
            if process.stdin_tx.send(data).await.is_ok() {
                return Ok(());
            }
        }

        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("[Ssh | Proc {}] Unable to send stdin to process", id),
        ))
    }

    async fn proc_resize_pty(
        &self,
        ctx: DistantCtx<Self::LocalData>,
        id: usize,
        size: PtySize,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Resizing pty of process {} to {}",
            ctx.connection_id, id, size
        );

        if let Some(process) = self.processes.read().await.get(&id) {
            if process.resize_tx.send(size).await.is_ok() {
                return Ok(());
            }
        }

        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("[Ssh | Proc {}] Unable to resize process", id),
        ))
    }

    async fn system_info(&self, ctx: DistantCtx<Self::LocalData>) -> io::Result<SystemInfo> {
        debug!("[Conn {}] Reading system information", ctx.connection_id);
        let current_dir = self
            .session
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

        Ok(SystemInfo {
            family,
            os: "".to_string(),
            arch: "".to_string(),
            current_dir,
            main_separator: if is_windows { '\\' } else { '/' },
        })
    }
}
