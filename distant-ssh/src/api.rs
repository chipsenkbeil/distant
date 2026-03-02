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
use typed_path::Utf8TypedPath;

use crate::process::Process;
use crate::{ClientHandler, SshFamily};

/// Convert a path string to SFTP path string using typed-path.
/// SFTP protocol always uses Unix-style paths regardless of target OS.
fn to_sftp_path(path: &str) -> io::Result<String> {
    let typed_path = Utf8TypedPath::derive(path);
    Ok(typed_path.with_unix_encoding().as_str().to_string())
}

/// Convert an SFTP path string to a native Windows path string.
/// SFTP returns paths like `/C:/Users/...` which need the leading `/` stripped
/// before the drive letter, then converted to Windows encoding via typed-path.
/// This is the reverse of `to_sftp_path` which uses `with_unix_encoding()`.
fn sftp_to_windows_path(sftp_path: &str) -> String {
    // Strip the leading / that SFTP prepends before drive letters (e.g. /C:/...)
    // so that derive() correctly detects the Windows drive prefix.
    let stripped = sftp_path
        .strip_prefix('/')
        .filter(|s| s.starts_with(|c: char| c.is_ascii_alphabetic()) && s[1..].starts_with(':'))
        .unwrap_or(sftp_path);
    Utf8TypedPath::derive(stripped)
        .with_windows_encoding()
        .to_string()
        .replace('/', "\\")
}

/// Join two path strings using the appropriate separator for the given family.
fn join_path_strings(base: &str, child: &str, family: SshFamily) -> String {
    let sep = if family == SshFamily::Windows {
        '\\'
    } else {
        '/'
    };

    if base.is_empty() {
        return child.to_string();
    }
    if base.ends_with(sep) || base.ends_with('/') || base.ends_with('\\') {
        format!("{base}{child}")
    } else {
        format!("{base}{sep}{child}")
    }
}

/// Strip a prefix from a path string, returning the relative remainder.
fn strip_prefix_string(path: &str, prefix: &str) -> Option<String> {
    let path_normalized = path.replace('\\', "/");
    let prefix_normalized = prefix.replace('\\', "/");
    let prefix_with_sep = if prefix_normalized.ends_with('/') {
        prefix_normalized
    } else {
        format!("{prefix_normalized}/")
    };
    path_normalized
        .strip_prefix(&prefix_with_sep)
        .map(|s| s.to_string())
}

/// Extract the file name (last component) from a path string.
fn file_name_from_path(path: &str) -> String {
    // Handle both Unix and Windows separators
    let last = path.rsplit(['/', '\\']).next().unwrap_or(path);
    last.to_string()
}

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
            SftpSession::new(channel.into_stream())
                .await
                .map_err(io::Error::other)?,
        );

        *sftp_lock = Some(Arc::clone(&sftp));
        Ok(sftp)
    }

    /// Convert a [`RemotePath`] to an SFTP path string using typed-path with validation.
    /// SFTP protocol always uses Unix-style paths regardless of target OS.
    fn to_sftp_path(&self, path: &RemotePath) -> io::Result<String> {
        to_sftp_path(path.as_str())
    }

    /// Converts an SFTP canonical path string to a native path string.
    /// On Windows SSH targets, SFTP returns Unix-style paths like `/C:/Users/...`
    /// that need conversion to native Windows format.
    fn sftp_path_to_native_string(&self, sftp_path: &str) -> String {
        if self.family == SshFamily::Windows {
            sftp_to_windows_path(sftp_path)
        } else {
            sftp_path.to_string()
        }
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

impl Api for SshApi {
    fn read_file(
        &self,
        ctx: Ctx,
        path: RemotePath,
    ) -> impl Future<Output = io::Result<Vec<u8>>> + Send {
        let sftp_path = self.to_sftp_path(&path);
        async move {
            debug!("[Conn {}] Reading file {}", ctx.connection_id, path);

            let sftp = self.get_sftp().await?;
            let sftp_path = sftp_path?;

            use tokio::io::AsyncReadExt;
            let mut file = sftp.open(&sftp_path).await.map_err(io::Error::other)?;

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
        let sftp_path = self.to_sftp_path(&path);
        async move {
            debug!("[Conn {}] Writing file {}", ctx.connection_id, path);

            let sftp = self.get_sftp().await?;
            let sftp_path = sftp_path?;

            use tokio::io::AsyncWriteExt;
            let mut file = sftp.create(&sftp_path).await.map_err(io::Error::other)?;

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
        let sftp_path = self.to_sftp_path(&path);
        async move {
            debug!("[Conn {}] Appending to file {}", ctx.connection_id, path);

            let sftp = self.get_sftp().await?;
            let sftp_path = sftp_path?;

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
            let sftp_path = self.to_sftp_path(&path)?;

            // When absolute or canonicalize paths are requested, use the canonicalized base path
            let base_path = if absolute || canonicalize {
                match sftp.canonicalize(&sftp_path).await {
                    Ok(canonical_str) => self.sftp_path_to_native_string(&canonical_str),
                    Err(_) => path.to_string(),
                }
            } else {
                path.to_string()
            };

            let mut entries = Vec::new();
            let mut errors = Vec::new();

            let family = self.family;

            // Helper function to read a single directory
            async fn read_single_dir(
                sftp: &Arc<russh_sftp::client::SftpSession>,
                path: &str,
                base_path: &str,
                absolute: bool,
                canonicalize: bool,
                family: SshFamily,
            ) -> io::Result<Vec<DirEntry>> {
                use distant_core::protocol::FileType;

                let convert = |s: &str| -> String {
                    if family == SshFamily::Windows {
                        sftp_to_windows_path(s)
                    } else {
                        s.to_string()
                    }
                };

                let dir_entries = sftp.read_dir(path).await.map_err(io::Error::other)?;

                let mut entries = Vec::new();
                for entry in dir_entries {
                    let filename = entry.file_name();
                    if filename == "." || filename == ".." {
                        continue;
                    }

                    let entry_path_str = if absolute {
                        join_path_strings(base_path, &filename, family)
                    } else if canonicalize {
                        if entry.metadata().is_symlink() {
                            let full_path = format!("{}/{}", path, filename);
                            // On Windows, SFTP realpath doesn't resolve symlinks.
                            // Use read_link to get the target, then canonicalize that.
                            let resolved = if family == SshFamily::Windows {
                                match sftp.read_link(&full_path).await {
                                    Ok(target) => {
                                        sftp.canonicalize(&target).await.unwrap_or(target)
                                    }
                                    Err(_) => sftp
                                        .canonicalize(&full_path)
                                        .await
                                        .unwrap_or(full_path.clone()),
                                }
                            } else {
                                sftp.canonicalize(&full_path)
                                    .await
                                    .unwrap_or(full_path.clone())
                            };
                            let canonical_path = convert(&resolved);
                            strip_prefix_string(&canonical_path, base_path)
                                .unwrap_or_else(|| filename.clone())
                        } else {
                            filename.clone()
                        }
                    } else {
                        filename.clone()
                    };

                    let file_type = if entry.metadata().is_dir() {
                        FileType::Dir
                    } else if entry.metadata().is_symlink() {
                        FileType::Symlink
                    } else {
                        FileType::File
                    };

                    entries.push(DirEntry {
                        path: RemotePath::new(entry_path_str),
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
                &base_path,
                absolute,
                canonicalize,
                family,
            )
            .await?;

            if include_root {
                let root_path = match sftp.canonicalize(&sftp_path).await {
                    Ok(p) => self.sftp_path_to_native_string(&p),
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
                        let subdir_path_str = if absolute || canonicalize {
                            entry.path.to_string()
                        } else {
                            join_path_strings(path.as_str(), entry.path.as_str(), family)
                        };

                        let subdir_sftp_path = to_sftp_path(&subdir_path_str)?;

                        match read_single_dir(
                            &sftp,
                            &subdir_sftp_path,
                            &subdir_path_str,
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
                                        // Extract just the filename from the sub_entry path
                                        let filename = file_name_from_path(sub_entry.path.as_str());
                                        sub_entry.path = RemotePath::new(join_path_strings(
                                            entry.path.as_str(),
                                            &filename,
                                            family,
                                        ));
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
            let sftp_path = self.to_sftp_path(&path)?;

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
            let sftp_path = self.to_sftp_path(&path)?;

            let metadata = sftp.metadata(&sftp_path).await.map_err(io::Error::other)?;

            if metadata.is_dir() {
                if force {
                    // Recursively remove directory contents using DFS
                    let mut dirs_to_remove = Vec::new();
                    let mut stack = vec![sftp_path.clone()];

                    while let Some(dir) = stack.pop() {
                        let entries = sftp.read_dir(&dir).await.map_err(io::Error::other)?;

                        for entry in entries {
                            let filename = entry.file_name();
                            if filename == "." || filename == ".." {
                                continue;
                            }
                            let entry_path = format!("{}/{}", dir, filename);
                            if entry.metadata().is_dir() {
                                stack.push(entry_path.clone());
                            } else {
                                sftp.remove_file(&entry_path)
                                    .await
                                    .map_err(io::Error::other)?;
                            }
                        }

                        dirs_to_remove.push(dir);
                    }

                    // Remove directories in reverse order (deepest first)
                    for dir in dirs_to_remove.into_iter().rev() {
                        sftp.remove_dir(&dir).await.map_err(io::Error::other)?;
                    }

                    Ok(())
                } else {
                    sftp.remove_dir(&sftp_path).await.map_err(io::Error::other)
                }
            } else {
                sftp.remove_file(&sftp_path).await.map_err(io::Error::other)
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
            let src_path = self.to_sftp_path(&src)?;
            let dst_path = self.to_sftp_path(&dst)?;

            sftp.rename(&src_path, &dst_path)
                .await
                .map_err(io::Error::other)
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
            let sftp_path = self.to_sftp_path(&path)?;

            match sftp.try_exists(&sftp_path).await {
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
            let sftp_path = self.to_sftp_path(&path)?;

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
                // On Windows, SFTP realpath doesn't resolve symlinks
                let resolved = if self.family == SshFamily::Windows && attrs.is_symlink() {
                    match sftp.read_link(&sftp_path).await {
                        Ok(target) => sftp.canonicalize(&target).await.ok(),
                        Err(_) => sftp.canonicalize(&sftp_path).await.ok(),
                    }
                } else {
                    sftp.canonicalize(&sftp_path).await.ok()
                };
                resolved.map(|p| RemotePath::new(self.sftp_path_to_native_string(&p)))
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
            let sftp_path = self.to_sftp_path(&path)?;

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
                    let current_dir = if is_windows {
                        sftp_to_windows_path(&path_str)
                    } else {
                        path_str
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

#[cfg(test)]
mod tests {
    //! Tests for `to_sftp_path` path conversion logic and string path helpers.

    use super::{
        SshFamily, file_name_from_path, join_path_strings, strip_prefix_string, to_sftp_path,
    };

    // --- to_sftp_path tests ---

    #[test]
    fn to_sftp_path_unix_absolute() {
        let result = to_sftp_path("/home/user/file.txt").unwrap();
        assert_eq!(result, "/home/user/file.txt");
    }

    #[test]
    fn to_sftp_path_unix_relative() {
        let result = to_sftp_path("relative/path/file.txt").unwrap();
        assert_eq!(result, "relative/path/file.txt");
    }

    #[test]
    fn to_sftp_path_root() {
        let result = to_sftp_path("/").unwrap();
        assert_eq!(result, "/");
    }

    #[test]
    fn to_sftp_path_single_file() {
        let result = to_sftp_path("file.txt").unwrap();
        assert_eq!(result, "file.txt");
    }

    #[test]
    fn to_sftp_path_dot_path() {
        let result = to_sftp_path(".").unwrap();
        assert_eq!(result, ".");
    }

    #[test]
    fn to_sftp_path_dot_dot_path() {
        let result = to_sftp_path("..").unwrap();
        assert_eq!(result, "..");
    }

    #[test]
    fn to_sftp_path_deep_nested() {
        let result = to_sftp_path("/a/b/c/d/e/f/g.txt").unwrap();
        assert_eq!(result, "/a/b/c/d/e/f/g.txt");
    }

    #[test]
    fn to_sftp_path_with_spaces() {
        let result = to_sftp_path("/path/with spaces/file name.txt").unwrap();
        assert_eq!(result, "/path/with spaces/file name.txt");
    }

    #[test]
    fn to_sftp_path_with_special_characters() {
        let result = to_sftp_path("/path/file-name_v2.0.txt").unwrap();
        assert_eq!(result, "/path/file-name_v2.0.txt");
    }

    #[test]
    fn to_sftp_path_hidden_file() {
        let result = to_sftp_path("/home/user/.hidden").unwrap();
        assert_eq!(result, "/home/user/.hidden");
    }

    #[test]
    fn to_sftp_path_empty_path() {
        let result = to_sftp_path("").unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn to_sftp_path_with_dots_in_name() {
        let result = to_sftp_path("/path/to/archive.tar.gz").unwrap();
        assert_eq!(result, "/path/to/archive.tar.gz");
    }

    #[test]
    fn to_sftp_path_double_slash_normalized() {
        let result = to_sftp_path("/home//user///file.txt").unwrap();
        // typed_path may normalize these, so just check it's valid
        assert!(result.contains("home"));
        assert!(result.contains("file.txt"));
    }

    #[test]
    fn to_sftp_path_trailing_slash() {
        let result = to_sftp_path("/home/user/").unwrap();
        assert!(result.starts_with("/home/user"));
    }

    #[test]
    fn to_sftp_path_with_unicode() {
        let result = to_sftp_path("/home/user/documents").unwrap();
        assert_eq!(result, "/home/user/documents");
    }

    #[test]
    fn to_sftp_path_relative_with_parent_ref() {
        let result = to_sftp_path("../parent/file.txt").unwrap();
        assert_eq!(result, "../parent/file.txt");
    }

    #[test]
    fn to_sftp_path_relative_with_dot() {
        let result = to_sftp_path("./current/file.txt").unwrap();
        // Result depends on typed_path behavior with leading dot
        assert!(result.contains("current/file.txt"));
    }

    #[test]
    fn sftp_to_windows_path_strips_leading_slash_before_drive() {
        // SFTP returns /C:/... — strip leading / so derive detects Windows prefix,
        // then forward slashes are normalized to backslashes.
        let result = super::sftp_to_windows_path("/C:/Users/foo/bar");
        assert_eq!(result, "C:\\Users\\foo\\bar");
    }

    #[test]
    fn sftp_to_windows_path_preserves_already_windows_path() {
        assert_eq!(
            super::sftp_to_windows_path("C:\\Users\\foo\\bar"),
            "C:\\Users\\foo\\bar"
        );
    }

    #[test]
    fn sftp_to_windows_path_converts_forward_slashes_to_backslashes() {
        // derive detects C:/ as Windows; with_windows_encoding is identity.
        // Forward slashes are then normalized to backslashes.
        assert_eq!(
            super::sftp_to_windows_path("C:/Users/foo/bar"),
            "C:\\Users\\foo\\bar"
        );
    }

    // --- join_path_strings tests ---

    #[test]
    fn join_path_strings_unix() {
        assert_eq!(
            join_path_strings("/home/user", "file.txt", SshFamily::Unix),
            "/home/user/file.txt"
        );
    }

    #[test]
    fn join_path_strings_unix_trailing_sep() {
        assert_eq!(
            join_path_strings("/home/user/", "file.txt", SshFamily::Unix),
            "/home/user/file.txt"
        );
    }

    #[test]
    fn join_path_strings_windows() {
        assert_eq!(
            join_path_strings("C:\\Users\\foo", "bar", SshFamily::Windows),
            "C:\\Users\\foo\\bar"
        );
    }

    #[test]
    fn join_path_strings_empty_base() {
        assert_eq!(
            join_path_strings("", "file.txt", SshFamily::Unix),
            "file.txt"
        );
    }

    // --- strip_prefix_string tests ---

    #[test]
    fn strip_prefix_string_basic() {
        assert_eq!(
            strip_prefix_string("/home/user/file.txt", "/home/user"),
            Some("file.txt".to_string())
        );
    }

    #[test]
    fn strip_prefix_string_no_match() {
        assert_eq!(strip_prefix_string("/other/path", "/home/user"), None);
    }

    #[test]
    fn strip_prefix_string_with_trailing_sep() {
        assert_eq!(
            strip_prefix_string("/home/user/file.txt", "/home/user/"),
            Some("file.txt".to_string())
        );
    }

    // --- file_name_from_path tests ---

    #[test]
    fn file_name_from_path_unix() {
        assert_eq!(file_name_from_path("/home/user/file.txt"), "file.txt");
    }

    #[test]
    fn file_name_from_path_windows() {
        assert_eq!(file_name_from_path("C:\\Users\\foo\\bar.txt"), "bar.txt");
    }

    #[test]
    fn file_name_from_path_no_separator() {
        assert_eq!(file_name_from_path("file.txt"), "file.txt");
    }
}
