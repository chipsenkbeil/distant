use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::io;
use std::sync::{Arc, Weak};

use async_once_cell::OnceCell;
use distant_core::protocol::{
    DirEntry, Environment, Metadata, PROTOCOL_VERSION, Permissions, ProcessId, PtySize, RemotePath,
    SearchId, SearchQuery, SetPermissionsOptions, SystemInfo, Version,
};
use distant_core::{Api, Ctx};
use log::*;
use russh::client::Handle;
use russh_sftp::client::SftpSession;
use tokio::sync::{Mutex, RwLock};

use crate::process::Process;
use crate::utils::{SSH_TIMEOUT_SECS, SftpPathBuf};
use crate::{ClientHandler, SshFamily};

/// Represents implementation of [`Api`] for SSH.
pub struct SshApi {
    /// Active SSH session handle.
    session: Handle<ClientHandler>,

    /// Lazy-cached SFTP session (created on first file operation).
    sftp: Arc<Mutex<Option<Arc<SftpSession>>>>,

    /// Global tracking of running processes by id.
    processes: Arc<RwLock<HashMap<ProcessId, Process>>>,

    /// Remote system family (Unix/Windows).
    family: SshFamily,

    /// Cached current working directory.
    cached_current_dir: OnceCell<String>,

    /// Cached username.
    cached_username: OnceCell<String>,

    /// Cached shell.
    cached_shell: OnceCell<String>,
}

impl SshApi {
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
            SftpSession::new_opts(channel.into_stream(), Some(SSH_TIMEOUT_SECS))
                .await
                .map_err(io::Error::other)?,
        );

        *sftp_lock = Some(Arc::clone(&sftp));
        Ok(sftp)
    }

    /// Convert a [`RemotePath`] (native format) to an [`SftpPathBuf`].
    fn sftp_path(&self, path: &RemotePath) -> SftpPathBuf {
        SftpPathBuf::from_remote(path, self.family)
    }

    /// Wrap an SFTP-returned string as an [`SftpPathBuf`].
    fn sftp_from_wire(&self, s: impl Into<String>) -> SftpPathBuf {
        SftpPathBuf::from_sftp(s, self.family)
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
            .map_err(|e| io::Error::other(format!("SFTP symlink_metadata '{path}': {e}")))?;

        if options.exclude_symlinks && metadata.is_symlink() {
            return Ok(None);
        }

        // Resolve symlinks if requested
        let (resolved_path, resolved_metadata) = if options.follow_symlinks && metadata.is_symlink()
        {
            let target = sftp
                .read_link(path)
                .await
                .map_err(|e| io::Error::other(format!("SFTP read_link '{path}': {e}")))?;
            let target_metadata = sftp
                .metadata(&target)
                .await
                .map_err(|e| io::Error::other(format!("SFTP metadata '{target}': {e}")))?;
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
            .map_err(|e| io::Error::other(format!("SFTP set_metadata '{resolved_path}': {e}")))?;

        if resolved_metadata.is_dir() {
            Ok(Some(resolved_path))
        } else {
            Ok(None)
        }
    }
}

impl Api for SshApi {
    fn read_file(
        &self,
        ctx: Ctx,
        path: RemotePath,
    ) -> impl Future<Output = io::Result<Vec<u8>>> + Send {
        let sftp_path = self.sftp_path(&path);
        async move {
            debug!("[Conn {}] Reading file {}", ctx.connection_id, path);

            let sftp = self.get_sftp().await?;

            use tokio::io::AsyncReadExt;
            let mut file = sftp
                .open(sftp_path.as_str())
                .await
                .map_err(|e| io::Error::other(format!("SFTP open '{}': {e}", sftp_path)))?;

            let mut contents = Vec::new();
            file.read_to_end(&mut contents).await?;

            Ok(contents)
        }
    }

    fn read_file_text(
        &self,
        ctx: Ctx,
        path: RemotePath,
    ) -> impl Future<Output = io::Result<String>> + Send {
        async move {
            let data = self.read_file(ctx, path).await?;
            String::from_utf8(data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
        }
    }

    fn write_file(
        &self,
        ctx: Ctx,
        path: RemotePath,
        data: Vec<u8>,
    ) -> impl Future<Output = io::Result<()>> + Send {
        let sftp_path = self.sftp_path(&path);
        async move {
            debug!("[Conn {}] Writing file {}", ctx.connection_id, path);

            let sftp = self.get_sftp().await?;

            use tokio::io::AsyncWriteExt;
            let mut file = sftp
                .create(sftp_path.as_str())
                .await
                .map_err(|e| io::Error::other(format!("SFTP create '{}': {e}", sftp_path)))?;

            file.write_all(&data).await?;
            file.flush().await?;

            Ok(())
        }
    }

    fn write_file_text(
        &self,
        ctx: Ctx,
        path: RemotePath,
        data: String,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async move { self.write_file(ctx, path, data.into_bytes()).await }
    }

    fn append_file(
        &self,
        ctx: Ctx,
        path: RemotePath,
        data: Vec<u8>,
    ) -> impl Future<Output = io::Result<()>> + Send {
        let sftp_path = self.sftp_path(&path);
        async move {
            debug!("[Conn {}] Appending to file {}", ctx.connection_id, path);

            let sftp = self.get_sftp().await?;

            use russh_sftp::protocol::OpenFlags;
            use tokio::io::AsyncWriteExt;

            let mut file = sftp
                .open_with_flags(
                    sftp_path.as_str(),
                    OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::APPEND,
                )
                .await
                .map_err(|e| {
                    io::Error::other(format!("SFTP open_with_flags '{}': {e}", sftp_path))
                })?;

            file.write_all(&data).await?;
            file.flush().await?;

            Ok(())
        }
    }

    fn append_file_text(
        &self,
        ctx: Ctx,
        path: RemotePath,
        data: String,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async move { self.append_file(ctx, path, data.into_bytes()).await }
    }

    fn read_dir(
        &self,
        ctx: Ctx,
        path: RemotePath,
        depth: usize,
        absolute: bool,
        canonicalize: bool,
        include_root: bool,
    ) -> impl Future<Output = io::Result<(Vec<DirEntry>, Vec<io::Error>)>> + Send {
        async move {
            debug!("[Conn {}] Reading directory {}", ctx.connection_id, path);

            let sftp = self.get_sftp().await?;
            let sftp_path = self.sftp_path(&path);

            // When absolute or canonicalize paths are requested, use the canonicalized base path
            let base_sftp = if absolute || canonicalize {
                match sftp.canonicalize(sftp_path.as_str()).await {
                    Ok(canonical_str) => self.sftp_from_wire(canonical_str),
                    Err(_) => sftp_path.clone(),
                }
            } else {
                sftp_path.clone()
            };

            let mut entries = Vec::new();
            let mut errors = Vec::new();

            let family = self.family;

            // Helper function to read a single directory
            async fn read_single_dir(
                sftp: &Arc<russh_sftp::client::SftpSession>,
                dir_path: &SftpPathBuf,
                base_path: &SftpPathBuf,
                absolute: bool,
                canonicalize: bool,
                family: SshFamily,
            ) -> io::Result<Vec<DirEntry>> {
                use distant_core::protocol::FileType;

                let dir_entries = sftp
                    .read_dir(dir_path.as_str())
                    .await
                    .map_err(|e| io::Error::other(format!("SFTP read_dir '{}': {e}", dir_path)))?;

                let mut entries = Vec::new();
                for entry in dir_entries {
                    let filename = entry.file_name();
                    if filename == "." || filename == ".." {
                        continue;
                    }

                    let entry_path_str = if absolute {
                        base_path.join(&filename).to_remote_path()
                    } else if canonicalize {
                        if entry.metadata().is_symlink() {
                            let full_sftp = dir_path.join(&filename);
                            // On Windows, SFTP realpath doesn't resolve symlinks.
                            // Use read_link to get the target, then canonicalize that.
                            let resolved = if family == SshFamily::Windows {
                                match sftp.read_link(full_sftp.as_str()).await {
                                    Ok(target) => {
                                        sftp.canonicalize(&target).await.unwrap_or(target)
                                    }
                                    Err(_) => sftp
                                        .canonicalize(full_sftp.as_str())
                                        .await
                                        .unwrap_or_else(|_| full_sftp.as_str().to_string()),
                                }
                            } else {
                                sftp.canonicalize(full_sftp.as_str())
                                    .await
                                    .unwrap_or_else(|_| full_sftp.as_str().to_string())
                            };
                            let resolved_sftp = SftpPathBuf::from_sftp(resolved, family);
                            match resolved_sftp.strip_prefix(base_path) {
                                Some(relative) => {
                                    SftpPathBuf::from_sftp(relative, family).to_remote_path()
                                }
                                None => RemotePath::new(filename.clone()),
                            }
                        } else {
                            RemotePath::new(filename.clone())
                        }
                    } else {
                        RemotePath::new(filename.clone())
                    };

                    let file_type = if entry.metadata().is_dir() {
                        FileType::Dir
                    } else if entry.metadata().is_symlink() {
                        FileType::Symlink
                    } else {
                        FileType::File
                    };

                    entries.push(DirEntry {
                        path: entry_path_str,
                        file_type,
                        depth: 1,
                    });
                }

                Ok(entries)
            }

            // Read root directory
            let mut root_entries = read_single_dir(
                &sftp,
                &sftp_path,
                &base_sftp,
                absolute,
                canonicalize,
                family,
            )
            .await?;

            if include_root {
                let root_path = match sftp.canonicalize(sftp_path.as_str()).await {
                    Ok(p) => self.sftp_from_wire(p).to_remote_path().to_string(),
                    Err(_) => path.to_string(),
                };

                entries.push(DirEntry {
                    path: RemotePath::new(root_path),
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
                    .filter(|e| {
                        e.file_type == distant_core::protocol::FileType::Dir && e.depth >= 1
                    })
                    .cloned()
                    .collect();
                let max_depth = if depth == 0 { usize::MAX } else { depth };

                while let Some(entry) = to_process.pop() {
                    if entry.file_type == distant_core::protocol::FileType::Dir
                        && entry.depth < max_depth
                    {
                        // Build SFTP path for the subdirectory
                        let subdir_sftp = if absolute || canonicalize {
                            SftpPathBuf::from_remote(&entry.path, family)
                        } else {
                            let entry_sftp = SftpPathBuf::from_remote(&entry.path, family);
                            sftp_path.join(entry_sftp.as_str())
                        };

                        // For absolute/canonicalize, the base is the entry's own path
                        let subdir_base = if absolute || canonicalize {
                            SftpPathBuf::from_remote(&entry.path, family)
                        } else {
                            subdir_sftp.clone()
                        };

                        match read_single_dir(
                            &sftp,
                            &subdir_sftp,
                            &subdir_base,
                            absolute,
                            canonicalize,
                            family,
                        )
                        .await
                        {
                            Ok(sub_entries) => {
                                for mut sub_entry in sub_entries {
                                    sub_entry.depth = entry.depth + 1;

                                    if !absolute && !canonicalize {
                                        // Build relative path: parent/filename in SFTP space,
                                        // then convert to native
                                        let filename = sub_entry.path.as_str().to_string();
                                        let parent = SftpPathBuf::from_remote(&entry.path, family);
                                        sub_entry.path = parent.join(&filename).to_remote_path();
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

            entries.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));

            Ok((entries, errors))
        }
    }

    fn create_dir(
        &self,
        ctx: Ctx,
        path: RemotePath,
        all: bool,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async move {
            debug!(
                "[Conn {}] Creating directory {} (all={})",
                ctx.connection_id, path, all
            );

            let sftp = self.get_sftp().await?;
            let sftp_path = self.sftp_path(&path);

            if all {
                use typed_path::{Utf8UnixPath, Utf8UnixPathBuf};

                let unix_path = Utf8UnixPath::new(sftp_path.as_str());
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
                sftp.create_dir(sftp_path.as_str())
                    .await
                    .map_err(|e| io::Error::other(format!("SFTP create_dir '{}': {e}", sftp_path)))
            }
        }
    }

    fn remove(
        &self,
        ctx: Ctx,
        path: RemotePath,
        force: bool,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async move {
            debug!(
                "[Conn {}] Removing {} (force={})",
                ctx.connection_id, path, force
            );

            let sftp = self.get_sftp().await?;
            let sftp_path = self.sftp_path(&path);

            let metadata = sftp
                .metadata(sftp_path.as_str())
                .await
                .map_err(|e| io::Error::other(format!("SFTP metadata '{}': {e}", sftp_path)))?;

            if metadata.is_dir() {
                if force {
                    // Recursively remove directory contents using DFS
                    let mut dirs_to_remove = Vec::new();
                    let mut stack = vec![sftp_path.into_string()];

                    while let Some(dir) = stack.pop() {
                        let entries = sftp
                            .read_dir(&dir)
                            .await
                            .map_err(|e| io::Error::other(format!("SFTP read_dir '{dir}': {e}")))?;

                        for entry in entries {
                            let filename = entry.file_name();
                            if filename == "." || filename == ".." {
                                continue;
                            }
                            let entry_path = format!("{}/{}", dir, filename);
                            if entry.metadata().is_dir() {
                                stack.push(entry_path.clone());
                            } else {
                                sftp.remove_file(&entry_path).await.map_err(|e| {
                                    io::Error::other(format!(
                                        "SFTP remove_file '{entry_path}': {e}"
                                    ))
                                })?;
                            }
                        }

                        dirs_to_remove.push(dir);
                    }

                    // Remove directories in reverse order (deepest first)
                    for dir in dirs_to_remove.into_iter().rev() {
                        sftp.remove_dir(&dir).await.map_err(|e| {
                            io::Error::other(format!("SFTP remove_dir '{dir}': {e}"))
                        })?;
                    }

                    Ok(())
                } else {
                    sftp.remove_dir(sftp_path.as_str()).await.map_err(|e| {
                        io::Error::other(format!("SFTP remove_dir '{}': {e}", sftp_path))
                    })
                }
            } else {
                sftp.remove_file(sftp_path.as_str())
                    .await
                    .map_err(|e| io::Error::other(format!("SFTP remove_file '{}': {e}", sftp_path)))
            }
        }
    }

    fn copy(
        &self,
        ctx: Ctx,
        src: RemotePath,
        dst: RemotePath,
    ) -> impl Future<Output = io::Result<()>> + Send {
        let family = self.family;
        let session = &self.session;
        async move {
            debug!("[Conn {}] Copying {} to {}", ctx.connection_id, src, dst);

            use crate::utils::execute_output;

            let src_str = src.as_str();
            let dst_str = dst.as_str();

            let command = if family == SshFamily::Windows {
                // Use `copy /Y` for files, `xcopy /E /I /Y` for directories.
                // `if exist "src\*"` returns true only for directories.
                format!(
                    "if exist \"{}\\*\" (xcopy /E /I /Y \"{}\" \"{}\") else (copy /Y \"{}\" \"{}\")",
                    src_str, src_str, dst_str, src_str, dst_str
                )
            } else {
                format!("cp -r \"{}\" \"{}\"", src_str, dst_str)
            };

            let output = execute_output(session, &command, None).await?;

            if !output.success {
                let stderr_str = String::from_utf8_lossy(&output.stderr);
                return Err(io::Error::other(format!("Copy failed: {}", stderr_str)));
            }

            Ok(())
        }
    }

    fn rename(
        &self,
        ctx: Ctx,
        src: RemotePath,
        dst: RemotePath,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async move {
            debug!("[Conn {}] Renaming {} to {}", ctx.connection_id, src, dst);

            let sftp = self.get_sftp().await?;
            let src_path = self.sftp_path(&src);
            let dst_path = self.sftp_path(&dst);

            sftp.rename(src_path.as_str(), dst_path.as_str())
                .await
                .map_err(|e| {
                    io::Error::other(format!("SFTP rename '{}' -> '{}': {e}", src_path, dst_path))
                })
        }
    }

    #[allow(unused_variables)]
    fn watch(
        &self,
        _ctx: Ctx,
        _path: RemotePath,
        _recursive: bool,
        _only: Vec<distant_core::protocol::ChangeKind>,
        _except: Vec<distant_core::protocol::ChangeKind>,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "File watching is not supported over SSH. Consider using proc_spawn for custom watchers.",
            ))
        }
    }

    #[allow(unused_variables)]
    fn unwatch(&self, _ctx: Ctx, _path: RemotePath) -> impl Future<Output = io::Result<()>> + Send {
        async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "File watching is not supported over SSH",
            ))
        }
    }

    fn exists(&self, ctx: Ctx, path: RemotePath) -> impl Future<Output = io::Result<bool>> + Send {
        async move {
            debug!(
                "[Conn {}] Checking existence of {}",
                ctx.connection_id, path
            );

            let sftp = self.get_sftp().await?;
            let sftp_path = self.sftp_path(&path);

            match sftp.try_exists(sftp_path.as_str()).await {
                Ok(exists) => Ok(exists),
                Err(_) => Ok(false),
            }
        }
    }

    fn metadata(
        &self,
        ctx: Ctx,
        path: RemotePath,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> impl Future<Output = io::Result<Metadata>> + Send {
        async move {
            debug!("[Conn {}] Getting metadata for {}", ctx.connection_id, path);

            let sftp = self.get_sftp().await?;
            let sftp_path = self.sftp_path(&path);

            let attrs = if resolve_file_type {
                sftp.metadata(sftp_path.as_str()).await
            } else {
                sftp.symlink_metadata(sftp_path.as_str()).await
            }
            .map_err(|e| io::Error::other(format!("SFTP metadata '{}': {e}", sftp_path)))?;

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
                // On Windows, SFTP realpath doesn't resolve symlinks
                let resolved = if self.family == SshFamily::Windows && attrs.is_symlink() {
                    match sftp.read_link(sftp_path.as_str()).await {
                        Ok(target) => sftp.canonicalize(&target).await.ok(),
                        Err(_) => sftp.canonicalize(sftp_path.as_str()).await.ok(),
                    }
                } else {
                    sftp.canonicalize(sftp_path.as_str()).await.ok()
                };
                resolved.map(|p| self.sftp_from_wire(p).to_remote_path())
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
    }

    fn set_permissions(
        &self,
        ctx: Ctx,
        path: RemotePath,
        permissions: Permissions,
        options: SetPermissionsOptions,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async move {
            debug!(
                "[Conn {}] Setting permissions for {}",
                ctx.connection_id, path
            );

            let sftp = self.get_sftp().await?;
            let sftp_path = self.sftp_path(&path);

            // Apply permissions to the root path
            let mut dirs = VecDeque::new();
            if let Some(dir_path) = self
                .apply_permissions(&sftp, sftp_path.as_str(), &permissions, &options)
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
    }

    #[allow(unused_variables)]
    fn search(
        &self,
        _ctx: Ctx,
        _query: SearchQuery,
    ) -> impl Future<Output = io::Result<SearchId>> + Send {
        async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Search is not supported over SSH. Use proc_spawn with find/grep commands instead.",
            ))
        }
    }

    #[allow(unused_variables)]
    fn cancel_search(
        &self,
        _ctx: Ctx,
        _id: SearchId,
    ) -> impl Future<Output = io::Result<()>> + Send {
        async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Search is not supported over SSH",
            ))
        }
    }

    fn proc_spawn(
        &self,
        ctx: Ctx,
        cmd: String,
        environment: Environment,
        current_dir: Option<RemotePath>,
        pty: Option<PtySize>,
    ) -> impl Future<Output = io::Result<ProcessId>> + Send {
        let session = &self.session;
        let processes = &self.processes;
        let global_processes = Arc::downgrade(processes);
        async move {
            debug!(
                "[Conn {}] Spawning {} {{environment: {:?}, current_dir: {:?}, pty: {:?}}}",
                ctx.connection_id, cmd, environment, current_dir, pty
            );

            use crate::process::{Process, SpawnResult, spawn_pty, spawn_simple};

            // Create cleanup closure that removes the process from tracking when it exits
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
                        session,
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
                        session,
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

            processes.write().await.insert(id, process);
            debug!(
                "[Conn {}] Spawned process {} successfully!",
                ctx.connection_id, id
            );

            Ok(id)
        }
    }

    fn proc_kill(&self, ctx: Ctx, id: ProcessId) -> impl Future<Output = io::Result<()>> + Send {
        let processes = &self.processes;
        async move {
            debug!("[Conn {}] Killing process {}", ctx.connection_id, id);

            let mut processes = processes.write().await;
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
    }

    fn proc_stdin(
        &self,
        ctx: Ctx,
        id: ProcessId,
        data: Vec<u8>,
    ) -> impl Future<Output = io::Result<()>> + Send {
        let processes = &self.processes;
        async move {
            debug!(
                "[Conn {}] Sending stdin to process {}",
                ctx.connection_id, id
            );

            let processes = processes.read().await;
            if let Some(process) = processes.get(&id) {
                if let Some(stdin_tx) = &process.stdin_tx {
                    stdin_tx.send(data).await.map_err(|_| {
                        io::Error::new(io::ErrorKind::BrokenPipe, "Stdin channel closed")
                    })
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
    }

    fn proc_resize_pty(
        &self,
        ctx: Ctx,
        id: ProcessId,
        size: PtySize,
    ) -> impl Future<Output = io::Result<()>> + Send {
        let processes = &self.processes;
        async move {
            debug!(
                "[Conn {}] Resizing pty for process {} to {:?}",
                ctx.connection_id, id, size
            );

            let processes = processes.read().await;
            if let Some(process) = processes.get(&id) {
                if let Some(resize_tx) = &process.resize_tx {
                    resize_tx.send(size).await.map_err(|_| {
                        io::Error::new(io::ErrorKind::BrokenPipe, "Resize channel closed")
                    })
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
    }

    fn system_info(&self, ctx: Ctx) -> impl Future<Output = io::Result<SystemInfo>> + Send {
        async move {
            debug!("[Conn {}] Reading system information", ctx.connection_id);

            let is_windows = self.family == SshFamily::Windows;

            let current_dir_str = self
                .cached_current_dir
                .get_or_try_init(async {
                    let sftp = self.get_sftp().await?;
                    let path_str = sftp.canonicalize(".").await.map_err(io::Error::other)?;

                    // Fix Windows paths: /C:/... -> C:\...
                    let current_dir = self.sftp_from_wire(path_str).to_remote_path().to_string();

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
                current_dir: RemotePath::new(current_dir_str),
                main_separator: if is_windows { '\\' } else { '/' },
                username,
                shell,
            })
        }
    }

    fn version(&self, ctx: Ctx) -> impl Future<Output = io::Result<Version>> + Send {
        async move {
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
}
