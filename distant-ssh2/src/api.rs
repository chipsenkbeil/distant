use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Weak};

use async_once_cell::OnceCell;
use async_trait::async_trait;
use distant_core::protocol::{
    DirEntry, Environment, Metadata, Permissions, ProcessId, PtySize, SearchId, SearchQuery,
    SetPermissionsOptions, SystemInfo, Version, PROTOCOL_VERSION,
};
use distant_core::{DistantApi, DistantCtx};
use log::*;
use russh::client::Handle;
use russh_sftp::client::SftpSession;
use tokio::sync::{Mutex, RwLock};
use typed_path::Utf8TypedPath;

use crate::process::Process;
use crate::{ClientHandler, SshFamily};

/// Represents implementation of [`DistantApi`] for SSH.
pub struct SshDistantApi {
    /// Active SSH session handle.
    session: Handle<ClientHandler>,

    /// Lazy-cached SFTP session (created on first file operation).
    sftp: Arc<Mutex<Option<Arc<SftpSession>>>>,

    /// Global tracking of running processes by id.
    processes: Arc<RwLock<HashMap<ProcessId, Process>>>,

    /// Remote system family (Unix/Windows).
    family: SshFamily,

    /// Cached current working directory.
    cached_current_dir: OnceCell<PathBuf>,

    /// Cached username.
    cached_username: OnceCell<String>,

    /// Cached shell.
    cached_shell: OnceCell<String>,
}

impl SshDistantApi {
    pub fn new(session: Handle<ClientHandler>, family: SshFamily) -> Self {
        Self {
            session,
            sftp: Arc::new(Mutex::new(None)),
            processes: Arc::new(RwLock::new(HashMap::new())),
            family,
            cached_current_dir: OnceCell::new(),
            cached_username: OnceCell::new(),
            cached_shell: OnceCell::new(),
        }
    }

    /// Get or create SFTP session (lazy initialization with caching).
    async fn get_sftp(&self) -> io::Result<Arc<SftpSession>> {
        let mut sftp_lock = self.sftp.lock().await;

        if let Some(sftp) = sftp_lock.as_ref() {
            return Ok(Arc::clone(sftp));
        }

        debug!("Creating new SFTP session");
        let channel = self
            .session
            .channel_open_session()
            .await
            .map_err(io::Error::other)?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(io::Error::other)?;

        let sftp = Arc::new(
            SftpSession::new(channel.into_stream())
                .await
                .map_err(io::Error::other)?,
        );

        *sftp_lock = Some(Arc::clone(&sftp));
        Ok(sftp)
    }

    /// Convert PathBuf to SFTP path string using typed-path with validation.
    /// SFTP protocol always uses Unix-style paths regardless of target OS.
    fn to_sftp_path(&self, path: PathBuf) -> io::Result<String> {
        let path_str = path.to_string_lossy();
        let typed_path = Utf8TypedPath::derive(&path_str);

        Ok(typed_path.with_unix_encoding().as_str().to_string())
    }

    /// Apply permissions to a single path via SFTP, reading current mode and merging.
    /// Returns the path if it is a directory (for recursive processing).
    async fn apply_permissions(
        &self,
        sftp: &SftpSession,
        path: &str,
        permissions: &Permissions,
        options: &SetPermissionsOptions,
    ) -> io::Result<Option<String>> {
        use russh_sftp::protocol::FileAttributes;

        let metadata = sftp
            .symlink_metadata(path)
            .await
            .map_err(io::Error::other)?;

        if options.exclude_symlinks && metadata.is_symlink() {
            return Ok(None);
        }

        // Resolve symlinks if requested
        let (resolved_path, resolved_metadata) = if options.follow_symlinks && metadata.is_symlink()
        {
            let target = sftp.read_link(path).await.map_err(io::Error::other)?;
            let target_metadata = sftp.metadata(&target).await.map_err(io::Error::other)?;
            (target, target_metadata)
        } else {
            (path.to_string(), metadata)
        };

        // Read current permissions and merge with the requested changes
        let perms = resolved_metadata.permissions();
        let mut current_mode: u32 = 0;
        if perms.owner_read {
            current_mode |= 0o400;
        }
        if perms.owner_write {
            current_mode |= 0o200;
        }
        if perms.owner_exec {
            current_mode |= 0o100;
        }
        if perms.group_read {
            current_mode |= 0o040;
        }
        if perms.group_write {
            current_mode |= 0o020;
        }
        if perms.group_exec {
            current_mode |= 0o010;
        }
        if perms.other_read {
            current_mode |= 0o004;
        }
        if perms.other_write {
            current_mode |= 0o002;
        }
        if perms.other_exec {
            current_mode |= 0o001;
        }
        let mut current_perms = Permissions::from_unix_mode(current_mode);
        current_perms.apply_from(permissions);
        let new_mode = current_perms.to_unix_mode();

        let new_attrs = FileAttributes {
            size: None,
            uid: None,
            user: None,
            gid: None,
            group: None,
            permissions: Some(new_mode),
            atime: None,
            mtime: None,
        };

        sftp.set_metadata(&resolved_path, new_attrs)
            .await
            .map_err(io::Error::other)?;

        if resolved_metadata.is_dir() {
            Ok(Some(resolved_path))
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl DistantApi for SshDistantApi {
    async fn read_file(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<Vec<u8>> {
        debug!("[Conn {}] Reading file {:?}", ctx.connection_id, path);

        let sftp = self.get_sftp().await?;
        let sftp_path = self.to_sftp_path(path)?;

        use tokio::io::AsyncReadExt;
        let mut file = sftp.open(&sftp_path).await.map_err(io::Error::other)?;

        let mut contents = Vec::new();
        file.read_to_end(&mut contents).await?;

        Ok(contents)
    }

    async fn read_file_text(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<String> {
        let data = self.read_file(ctx, path).await?;
        String::from_utf8(data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    async fn write_file(&self, ctx: DistantCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()> {
        debug!("[Conn {}] Writing file {:?}", ctx.connection_id, path);

        let sftp = self.get_sftp().await?;
        let sftp_path = self.to_sftp_path(path)?;

        use tokio::io::AsyncWriteExt;
        let mut file = sftp.create(&sftp_path).await.map_err(io::Error::other)?;

        file.write_all(&data).await?;
        file.flush().await?;

        Ok(())
    }

    async fn write_file_text(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        data: String,
    ) -> io::Result<()> {
        self.write_file(ctx, path, data.into_bytes()).await
    }

    async fn append_file(&self, ctx: DistantCtx, path: PathBuf, data: Vec<u8>) -> io::Result<()> {
        debug!("[Conn {}] Appending to file {:?}", ctx.connection_id, path);

        let sftp = self.get_sftp().await?;
        let sftp_path = self.to_sftp_path(path)?;

        use russh_sftp::protocol::OpenFlags;
        use tokio::io::AsyncWriteExt;

        let mut file = sftp
            .open_with_flags(
                &sftp_path,
                OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::APPEND,
            )
            .await
            .map_err(io::Error::other)?;

        file.write_all(&data).await?;
        file.flush().await?;

        Ok(())
    }

    async fn append_file_text(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        data: String,
    ) -> io::Result<()> {
        self.append_file(ctx, path, data.into_bytes()).await
    }

    async fn read_dir(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> io::Result<(Vec<DirEntry>, Vec<io::Error>)> {
        debug!("[Conn {}] Reading directory {:?}", ctx.connection_id, path);

        let sftp = self.get_sftp().await?;
        let sftp_path = self.to_sftp_path(path.clone())?;

        // When absolute or canonicalize paths are requested, use the canonicalized base path
        let base_path = if absolute || canonicalize {
            match sftp.canonicalize(&sftp_path).await {
                Ok(canonical_str) => PathBuf::from(canonical_str),
                Err(_) => path.clone(),
            }
        } else {
            path.clone()
        };

        let mut entries = Vec::new();
        let mut errors = Vec::new();

        // Helper function to read a single directory
        async fn read_single_dir(
            sftp: &Arc<russh_sftp::client::SftpSession>,
            path: &str,
            base_path: &PathBuf,
            absolute: bool,
            canonicalize: bool,
        ) -> io::Result<Vec<DirEntry>> {
            use distant_core::protocol::FileType;

            let dir_entries = sftp.read_dir(path).await.map_err(io::Error::other)?;

            let mut entries = Vec::new();
            for entry in dir_entries {
                let filename = entry.file_name();
                if filename == "." || filename == ".." {
                    continue;
                }

                let entry_path = if absolute {
                    base_path.join(&filename)
                } else if canonicalize {
                    if entry.metadata().is_symlink() {
                        let full_path = format!("{}/{}", path, filename);
                        match sftp.canonicalize(&full_path).await {
                            Ok(canonical_str) => {
                                let canonical_path = PathBuf::from(canonical_str);
                                canonical_path
                                    .strip_prefix(base_path)
                                    .map(|p| p.to_path_buf())
                                    .unwrap_or_else(|_| PathBuf::from(&filename))
                            }
                            Err(_) => PathBuf::from(&filename),
                        }
                    } else {
                        PathBuf::from(&filename)
                    }
                } else {
                    PathBuf::from(&filename)
                };

                let file_type = if entry.metadata().is_dir() {
                    FileType::Dir
                } else if entry.metadata().is_symlink() {
                    FileType::Symlink
                } else {
                    FileType::File
                };

                entries.push(DirEntry {
                    path: entry_path,
                    file_type,
                    depth: 1,
                });
            }

            Ok(entries)
        }

        // Read root directory
        let mut root_entries =
            read_single_dir(&sftp, &sftp_path, &base_path, absolute, canonicalize).await?;

        if include_root {
            let root_path = match sftp.canonicalize(&sftp_path).await {
                Ok(p) => PathBuf::from(p),
                Err(_) => path.clone(),
            };

            entries.push(DirEntry {
                path: root_path,
                file_type: distant_core::protocol::FileType::Dir,
                depth: 0,
            });
        }
        entries.append(&mut root_entries);

        // Implement recursive directory reading for depth > 1 or depth == 0 (unlimited)
        if depth == 0 || depth > 1 {
            // Seed the work queue with directories from the initial listing
            let mut to_process: Vec<DirEntry> = entries
                .iter()
                .filter(|e| e.file_type == distant_core::protocol::FileType::Dir && e.depth >= 1)
                .cloned()
                .collect();
            let max_depth = if depth == 0 { usize::MAX } else { depth };

            while let Some(entry) = to_process.pop() {
                if entry.file_type == distant_core::protocol::FileType::Dir
                    && entry.depth < max_depth
                {
                    let subdir_path = if absolute || canonicalize {
                        entry.path.clone()
                    } else {
                        path.join(&entry.path)
                    };

                    let subdir_sftp_path = self.to_sftp_path(subdir_path.clone())?;

                    match read_single_dir(
                        &sftp,
                        &subdir_sftp_path,
                        &subdir_path,
                        absolute,
                        canonicalize,
                    )
                    .await
                    {
                        Ok(sub_entries) => {
                            for mut sub_entry in sub_entries {
                                sub_entry.depth = entry.depth + 1;

                                if !absolute && !canonicalize {
                                    sub_entry.path =
                                        entry.path.join(sub_entry.path.file_name().unwrap());
                                }

                                to_process.push(sub_entry.clone());
                                entries.push(sub_entry);
                            }
                        }
                        Err(e) => {
                            errors.push(e);
                        }
                    }
                }
            }
        }

        entries.sort_by(|a, b| a.path.cmp(&b.path));

        Ok((entries, errors))
    }

    async fn create_dir(&self, ctx: DistantCtx, path: PathBuf, all: bool) -> io::Result<()> {
        debug!(
            "[Conn {}] Creating directory {:?} (all={})",
            ctx.connection_id, path, all
        );

        let sftp = self.get_sftp().await?;
        let sftp_path = self.to_sftp_path(path.clone())?;

        if all {
            use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

            let unix_path = Utf8UnixPath::new(&sftp_path);
            let mut current_path = Utf8UnixPathBuf::new();

            for component in unix_path.components() {
                use typed_path::Utf8Component;
                match component {
                    c if c.is_root() => {
                        current_path = Utf8UnixPathBuf::from("/");
                    }
                    c if c.is_normal() => {
                        let part = c.as_str();
                        current_path.push(part);
                        let current_path_str = current_path.as_str();

                        if let Err(e) = sftp.create_dir(current_path_str).await {
                            debug!("create_dir error for {}: {:?}", current_path_str, e);
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        } else {
            sftp.create_dir(&sftp_path).await.map_err(io::Error::other)
        }
    }

    async fn remove(&self, ctx: DistantCtx, path: PathBuf, force: bool) -> io::Result<()> {
        debug!(
            "[Conn {}] Removing {:?} (force={})",
            ctx.connection_id, path, force
        );

        let sftp = self.get_sftp().await?;
        let sftp_path = self.to_sftp_path(path)?;

        let metadata = sftp.metadata(&sftp_path).await.map_err(io::Error::other)?;

        if metadata.is_dir() {
            if force {
                let entries = sftp.read_dir(&sftp_path).await.map_err(io::Error::other)?;

                for entry in entries {
                    let filename = entry.file_name();
                    if filename != "." && filename != ".." {
                        let entry_path = format!("{}/{}", sftp_path, filename);
                        if entry.metadata().is_dir() {
                            sftp.remove_dir(&entry_path)
                                .await
                                .map_err(io::Error::other)?;
                        } else {
                            sftp.remove_file(&entry_path)
                                .await
                                .map_err(io::Error::other)?;
                        }
                    }
                }
            }
            sftp.remove_dir(&sftp_path).await.map_err(io::Error::other)
        } else {
            sftp.remove_file(&sftp_path).await.map_err(io::Error::other)
        }
    }

    async fn copy(&self, ctx: DistantCtx, src: PathBuf, dst: PathBuf) -> io::Result<()> {
        debug!(
            "[Conn {}] Copying {:?} to {:?}",
            ctx.connection_id, src, dst
        );

        use crate::utils::execute_output;

        let src_str = src.to_string_lossy();
        let dst_str = dst.to_string_lossy();

        let command = if self.family == SshFamily::Windows {
            format!("xcopy /E /I /Y \"{}\" \"{}\"", src_str, dst_str)
        } else {
            format!("cp -r \"{}\" \"{}\"", src_str, dst_str)
        };

        let output = execute_output(&self.session, &command, None).await?;

        if !output.success {
            let stderr_str = String::from_utf8_lossy(&output.stderr);
            return Err(io::Error::other(format!("Copy failed: {}", stderr_str)));
        }

        Ok(())
    }

    async fn rename(&self, ctx: DistantCtx, src: PathBuf, dst: PathBuf) -> io::Result<()> {
        debug!(
            "[Conn {}] Renaming {:?} to {:?}",
            ctx.connection_id, src, dst
        );

        let sftp = self.get_sftp().await?;
        let src_path = self.to_sftp_path(src)?;
        let dst_path = self.to_sftp_path(dst)?;

        sftp.rename(&src_path, &dst_path)
            .await
            .map_err(io::Error::other)
    }

    async fn watch(
        &self,
        _ctx: DistantCtx,
        _path: PathBuf,
        _recursive: bool,
        _only: Vec<distant_core::protocol::ChangeKind>,
        _except: Vec<distant_core::protocol::ChangeKind>,
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "File watching is not supported over SSH. Consider using proc_spawn for custom watchers.",
        ))
    }

    async fn unwatch(&self, _ctx: DistantCtx, _path: PathBuf) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "File watching is not supported over SSH",
        ))
    }

    async fn exists(&self, ctx: DistantCtx, path: PathBuf) -> io::Result<bool> {
        debug!(
            "[Conn {}] Checking existence of {:?}",
            ctx.connection_id, path
        );

        let sftp = self.get_sftp().await?;
        let sftp_path = self.to_sftp_path(path)?;

        match sftp.try_exists(&sftp_path).await {
            Ok(exists) => Ok(exists),
            Err(_) => Ok(false),
        }
    }

    async fn metadata(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> io::Result<Metadata> {
        debug!(
            "[Conn {}] Getting metadata for {:?}",
            ctx.connection_id, path
        );

        let sftp = self.get_sftp().await?;
        let sftp_path = self.to_sftp_path(path.clone())?;

        let attrs = if resolve_file_type {
            sftp.metadata(&sftp_path).await
        } else {
            sftp.symlink_metadata(&sftp_path).await
        }
        .map_err(io::Error::other)?;

        use std::time::SystemTime;

        use distant_core::protocol::FileType;

        let file_type = if attrs.is_dir() {
            FileType::Dir
        } else if attrs.is_symlink() {
            FileType::Symlink
        } else {
            FileType::File
        };

        let canonical_path = if canonicalize {
            match sftp.canonicalize(&sftp_path).await {
                Ok(p) => Some(PathBuf::from(p)),
                Err(_) => None,
            }
        } else {
            None
        };

        let systemtime_to_secs = |st: SystemTime| -> u64 {
            st.duration_since(SystemTime::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs())
                .unwrap_or(0)
        };

        let perms = attrs.permissions();
        let unix_metadata = Some(distant_core::protocol::UnixMetadata {
            owner_read: perms.owner_read,
            owner_write: perms.owner_write,
            owner_exec: perms.owner_exec,
            group_read: perms.group_read,
            group_write: perms.group_write,
            group_exec: perms.group_exec,
            other_read: perms.other_read,
            other_write: perms.other_write,
            other_exec: perms.other_exec,
        });

        Ok(Metadata {
            canonicalized_path: canonical_path,
            file_type,
            len: attrs.len(),
            readonly: unix_metadata
                .as_ref()
                .map(|u| !u.owner_write && !u.group_write && !u.other_write)
                .unwrap_or(true),
            accessed: attrs.accessed().ok().map(systemtime_to_secs),
            created: None, // SFTP doesn't provide creation time
            modified: attrs.modified().ok().map(systemtime_to_secs),
            unix: unix_metadata,
            windows: None,
        })
    }

    async fn set_permissions(
        &self,
        ctx: DistantCtx,
        path: PathBuf,
        permissions: Permissions,
        options: SetPermissionsOptions,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Setting permissions for {:?}",
            ctx.connection_id, path
        );

        let sftp = self.get_sftp().await?;
        let sftp_path = self.to_sftp_path(path)?;

        // Apply permissions to the root path
        let mut dirs = VecDeque::new();
        if let Some(dir_path) = self
            .apply_permissions(&sftp, &sftp_path, &permissions, &options)
            .await?
        {
            dirs.push_back(dir_path);
        }

        // Recursively apply to directory contents via BFS
        if options.recursive {
            while let Some(dir) = dirs.pop_front() {
                let dir_entries = sftp.read_dir(&dir).await.map_err(io::Error::other)?;
                for entry in dir_entries {
                    let filename = entry.file_name();
                    if filename == "." || filename == ".." {
                        continue;
                    }

                    let entry_path = format!("{}/{}", dir, filename);
                    match self
                        .apply_permissions(&sftp, &entry_path, &permissions, &options)
                        .await
                    {
                        Ok(Some(sub_dir)) => dirs.push_back(sub_dir),
                        Ok(None) => {}
                        Err(e) => {
                            warn!("Failed to set permissions on {}: {}", entry_path, e);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn search(&self, _ctx: DistantCtx, _query: SearchQuery) -> io::Result<SearchId> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Search is not supported over SSH. Use proc_spawn with find/grep commands instead.",
        ))
    }

    async fn cancel_search(&self, _ctx: DistantCtx, _id: SearchId) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Search is not supported over SSH",
        ))
    }

    async fn proc_spawn(
        &self,
        ctx: DistantCtx,
        cmd: String,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> io::Result<ProcessId> {
        debug!(
            "[Conn {}] Spawning {} {{environment: {:?}, current_dir: {:?}, pty: {:?}}}",
            ctx.connection_id, cmd, environment, current_dir, pty
        );

        use crate::process::{spawn_pty, spawn_simple, Process, SpawnResult};

        // Create cleanup closure that removes the process from tracking when it exits
        let global_processes = Arc::downgrade(&self.processes);
        let make_cleanup = |processes_ref: Weak<RwLock<HashMap<ProcessId, Process>>>| {
            move |id: ProcessId| async move {
                if let Some(processes) = processes_ref.upgrade() {
                    processes.write().await.remove(&id);
                }
            }
        };

        let SpawnResult {
            id,
            stdin,
            killer,
            resizer,
        } = match pty {
            None => {
                spawn_simple(
                    &self.session,
                    &cmd,
                    environment,
                    current_dir,
                    ctx.reply.clone_reply(),
                    make_cleanup(global_processes),
                )
                .await?
            }
            Some(size) => {
                spawn_pty(
                    &self.session,
                    &cmd,
                    environment,
                    current_dir,
                    size,
                    ctx.reply.clone_reply(),
                    make_cleanup(global_processes),
                )
                .await?
            }
        };

        let process = Process {
            id,
            stdin_tx: Some(stdin),
            kill_tx: Some(killer),
            resize_tx: Some(resizer),
        };

        self.processes.write().await.insert(id, process);
        debug!(
            "[Conn {}] Spawned process {} successfully!",
            ctx.connection_id, id
        );

        Ok(id)
    }

    async fn proc_kill(&self, ctx: DistantCtx, id: ProcessId) -> io::Result<()> {
        debug!("[Conn {}] Killing process {}", ctx.connection_id, id);

        let mut processes = self.processes.write().await;
        if let Some(process) = processes.get_mut(&id) {
            if let Some(killer) = process.kill_tx.take() {
                let _ = killer.send(()).await;
            }
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Process {} not found", id),
            ))
        }
    }

    async fn proc_stdin(&self, ctx: DistantCtx, id: ProcessId, data: Vec<u8>) -> io::Result<()> {
        debug!(
            "[Conn {}] Sending stdin to process {}",
            ctx.connection_id, id
        );

        let processes = self.processes.read().await;
        if let Some(process) = processes.get(&id) {
            if let Some(stdin_tx) = &process.stdin_tx {
                stdin_tx
                    .send(data)
                    .await
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Stdin channel closed"))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Process stdin is closed",
                ))
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Process {} not found", id),
            ))
        }
    }

    async fn proc_resize_pty(
        &self,
        ctx: DistantCtx,
        id: ProcessId,
        size: PtySize,
    ) -> io::Result<()> {
        debug!(
            "[Conn {}] Resizing pty for process {} to {:?}",
            ctx.connection_id, id, size
        );

        let processes = self.processes.read().await;
        if let Some(process) = processes.get(&id) {
            if let Some(resize_tx) = &process.resize_tx {
                resize_tx
                    .send(size)
                    .await
                    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Resize channel closed"))
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Process is not a PTY",
                ))
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Process {} not found", id),
            ))
        }
    }

    async fn system_info(&self, ctx: DistantCtx) -> io::Result<SystemInfo> {
        debug!("[Conn {}] Reading system information", ctx.connection_id);

        let is_windows = self.family == SshFamily::Windows;

        let current_dir = self
            .cached_current_dir
            .get_or_try_init(async {
                let sftp = self.get_sftp().await?;
                let path_str = sftp.canonicalize(".").await.map_err(io::Error::other)?;
                let current_dir = PathBuf::from(&path_str);

                // Fix Windows paths: /C:/... -> C:\...
                let current_dir = if is_windows {
                    current_dir
                        .to_str()
                        .and_then(crate::utils::convert_to_windows_path_string)
                        .map(PathBuf::from)
                        .unwrap_or(current_dir)
                } else {
                    current_dir
                };

                Result::<_, io::Error>::Ok(current_dir)
            })
            .await?
            .clone();

        let session = &self.session;
        let username = self
            .cached_username
            .get_or_try_init(crate::utils::query_username(session, is_windows))
            .await?
            .clone();

        let shell = self
            .cached_shell
            .get_or_try_init(crate::utils::query_shell(session, is_windows))
            .await?
            .clone();

        Ok(SystemInfo {
            family: match self.family {
                SshFamily::Unix => "unix".to_string(),
                SshFamily::Windows => "windows".to_string(),
            },
            os: if is_windows {
                "windows".to_string()
            } else {
                // Complex to determine over SSH without additional platform-specific commands
                String::new()
            },
            // Complex to determine over SSH without additional platform-specific commands
            arch: String::new(),
            current_dir,
            main_separator: if is_windows { '\\' } else { '/' },
            username,
            shell,
        })
    }

    async fn version(&self, ctx: DistantCtx) -> io::Result<Version> {
        debug!("[Conn {}] Querying capabilities", ctx.connection_id);

        let capabilities = vec![
            Version::CAP_EXEC.to_string(),
            Version::CAP_FS_IO.to_string(),
            Version::CAP_SYS_INFO.to_string(),
        ];

        use distant_core::protocol::semver;

        let mut server_version: semver::Version = env!("CARGO_PKG_VERSION")
            .parse()
            .map_err(io::Error::other)?;

        if server_version.build.is_empty() {
            server_version.build =
                semver::BuildMetadata::new(env!("CARGO_PKG_NAME")).map_err(io::Error::other)?;
        } else {
            let raw_build_str = format!(
                "{}.{}",
                server_version.build.as_str(),
                env!("CARGO_PKG_NAME")
            );
            server_version.build =
                semver::BuildMetadata::new(&raw_build_str).map_err(io::Error::other)?;
        }

        Ok(Version {
            server_version,
            protocol_version: PROTOCOL_VERSION,
            capabilities,
        })
    }
}
