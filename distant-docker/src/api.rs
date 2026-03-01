//! Docker implementation of the distant [`Api`] trait.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use async_once_cell::OnceCell;
use bollard::Docker;
use distant_core::protocol::{
    ChangeKind, DirEntry, Environment, FileType, Metadata, PROTOCOL_VERSION, Permissions,
    ProcessId, PtySize, SearchId, SearchQuery, SearchQueryMatch, SearchQueryTarget,
    SetPermissionsOptions, SystemInfo, UnixMetadata, Version,
};
use distant_core::{Api, Ctx};
use tokio::sync::RwLock;

use crate::process::{self, Process, SpawnResult};
use crate::search;
use crate::utils::{self, SearchTools};
use crate::{DockerFamily, DockerOpts};

/// Docker implementation of the distant [`Api`] trait.
///
/// Translates distant operations to Docker API calls using a combination of the tar archive
/// API (for file I/O) and container exec (for process and filesystem operations).
pub struct DockerApi {
    /// Docker client handle.
    client: Docker,

    /// Container name or ID.
    container: String,

    /// Detected OS family.
    family: DockerFamily,

    /// Connection options.
    opts: DockerOpts,

    /// Global tracking of running processes.
    processes: Arc<RwLock<HashMap<ProcessId, Process>>>,

    /// Active search processes keyed by search ID.
    #[allow(dead_code)]
    searches: Arc<RwLock<HashMap<SearchId, ProcessId>>>,

    /// Detected search tools available in the container.
    search_tools: SearchTools,

    /// Cached current working directory.
    cached_current_dir: OnceCell<PathBuf>,

    /// Cached username.
    cached_username: OnceCell<String>,

    /// Cached shell.
    cached_shell: OnceCell<String>,
}

impl DockerApi {
    /// Creates a new `DockerApi`, probing the container for available tools.
    pub async fn new(
        client: Docker,
        container: String,
        family: DockerFamily,
        opts: DockerOpts,
    ) -> Self {
        let search_tools = utils::probe_search_tools(&client, &container, family).await;

        Self {
            client,
            container,
            family,
            opts,
            processes: Arc::new(RwLock::new(HashMap::new())),
            searches: Arc::new(RwLock::new(HashMap::new())),
            search_tools,
            cached_current_dir: OnceCell::new(),
            cached_username: OnceCell::new(),
            cached_shell: OnceCell::new(),
        }
    }

    /// Returns the user override from options, if any.
    fn user(&self) -> Option<&str> {
        self.opts.user.as_deref()
    }

    /// Execute a command in the container and return its output.
    async fn run_cmd(&self, cmd: &[&str]) -> io::Result<utils::ExecOutput> {
        utils::execute_output(&self.client, &self.container, cmd, self.user()).await
    }

    /// Execute a command and return its stdout as a string, or error if it fails.
    async fn run_cmd_stdout(&self, cmd: &[&str]) -> io::Result<String> {
        let output = self.run_cmd(cmd).await?;
        if output.success() {
            Ok(output.stdout_str())
        } else {
            Err(io::Error::other(format!(
                "Command failed (exit {}): {}",
                output.exit_code,
                output.stderr_str()
            )))
        }
    }

    /// Execute a shell command string using the appropriate shell for the container OS.
    ///
    /// Uses `sh -c` on Unix and `cmd /c` on Windows.
    async fn run_shell_cmd(&self, script: &str) -> io::Result<utils::ExecOutput> {
        match self.family {
            DockerFamily::Unix => self.run_cmd(&["sh", "-c", script]).await,
            DockerFamily::Windows => self.run_cmd(&["cmd", "/c", script]).await,
        }
    }

    /// Execute a shell command and return stdout, or error if the command fails.
    async fn run_shell_cmd_stdout(&self, script: &str) -> io::Result<String> {
        let output = self.run_shell_cmd(script).await?;
        if output.success() {
            Ok(output.stdout_str())
        } else {
            Err(io::Error::other(format!(
                "Command failed (exit {}): {}",
                output.exit_code,
                output.stderr_str()
            )))
        }
    }

    /// Execute a command in the container as a specific user.
    async fn run_cmd_as(&self, user: &str, cmd: &[&str]) -> io::Result<utils::ExecOutput> {
        utils::execute_output(&self.client, &self.container, cmd, Some(user)).await
    }

    /// Execute a shell command string as a specific user.
    ///
    /// Uses `sh -c` on Unix and `cmd /c` on Windows.
    async fn run_shell_cmd_as(&self, user: &str, script: &str) -> io::Result<utils::ExecOutput> {
        match self.family {
            DockerFamily::Unix => self.run_cmd_as(user, &["sh", "-c", script]).await,
            DockerFamily::Windows => self.run_cmd_as(user, &["cmd", "/c", script]).await,
        }
    }

    /// Perform a search using the Docker tar API as a fallback.
    ///
    /// On Windows nanoserver, exec-based search tools (`findstr`, `dir /s /b`) cannot
    /// access paths created via the Docker tar API due to `ContainerUser` permission
    /// restrictions. This method reads directory listings and file contents through
    /// the Docker archive API and performs matching in Rust.
    async fn tar_based_search(&self, query: &SearchQuery) -> Vec<SearchQueryMatch> {
        use distant_core::protocol::{
            SearchQueryContentsMatch, SearchQueryMatchData, SearchQueryPathMatch,
            SearchQuerySubmatch,
        };

        let mut matches = Vec::new();

        let path = query
            .paths
            .first()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());

        // List all entries in the search directory via tar
        let entries = match utils::tar_list_dir(&self.client, &self.container, &path).await {
            Ok(e) => e,
            Err(_) => return matches,
        };

        match query.target {
            SearchQueryTarget::Path => {
                for (_entry_type, entry_path, _size, _mtime) in &entries {
                    let filename = std::path::Path::new(entry_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();

                    if search::condition_matches(&query.condition, &filename) {
                        // Reconstruct the full container path.
                        // tar_list_dir returns paths like "dirname/file.txt" — strip
                        // the first component and any leading separator so Path::join
                        // treats the remainder as relative (not absolute).
                        let relative = entry_path
                            .strip_prefix(
                                std::path::Path::new(entry_path)
                                    .components()
                                    .next()
                                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                                    .unwrap_or_default()
                                    .as_str(),
                            )
                            .unwrap_or(entry_path)
                            .trim_start_matches(['/', '\\']);
                        let full_path = std::path::Path::new(&path).join(relative);
                        matches.push(SearchQueryMatch::Path(SearchQueryPathMatch {
                            path: full_path,
                            submatches: vec![SearchQuerySubmatch {
                                r#match: SearchQueryMatchData::Text(filename),
                                start: 0,
                                end: 0,
                            }],
                        }));
                    }
                }
            }
            SearchQueryTarget::Contents => {
                for (entry_type, entry_path, _size, _mtime) in &entries {
                    // Skip directories — only search file contents
                    if *entry_type == tar::EntryType::Directory {
                        continue;
                    }

                    // Reconstruct the full container path for this file
                    let relative = entry_path
                        .strip_prefix(
                            std::path::Path::new(entry_path)
                                .components()
                                .next()
                                .map(|c| c.as_os_str().to_string_lossy().to_string())
                                .unwrap_or_default()
                                .as_str(),
                        )
                        .unwrap_or(entry_path)
                        .trim_start_matches(['/', '\\']);
                    let full_path = std::path::Path::new(&path).join(relative);
                    let full_path_str = full_path.to_string_lossy().to_string();

                    // Read file contents via tar API
                    let data =
                        match utils::tar_read_file(&self.client, &self.container, &full_path_str)
                            .await
                        {
                            Ok(d) => d,
                            Err(_) => continue,
                        };

                    let text = String::from_utf8_lossy(&data);
                    for (line_num, line) in text.lines().enumerate() {
                        if search::condition_matches(&query.condition, line) {
                            matches.push(SearchQueryMatch::Contents(SearchQueryContentsMatch {
                                path: full_path.clone(),
                                lines: SearchQueryMatchData::Text(line.to_string()),
                                line_number: (line_num + 1) as u64,
                                absolute_offset: 0,
                                submatches: vec![SearchQuerySubmatch {
                                    r#match: SearchQueryMatchData::Text(line.to_string()),
                                    start: 0,
                                    end: 0,
                                }],
                            }));
                        }
                    }
                }
            }
        }

        matches
    }
}

impl Api for DockerApi {
    fn version(&self, _ctx: Ctx) -> impl std::future::Future<Output = io::Result<Version>> + Send {
        async move {
            let mut capabilities = vec![
                Version::CAP_EXEC.to_string(),
                Version::CAP_FS_IO.to_string(),
                Version::CAP_SYS_INFO.to_string(),
            ];

            // Only advertise search if we have tools
            if self.search_tools.has_any() {
                capabilities.push(Version::CAP_FS_SEARCH.to_string());
            }

            // Advertise permissions for Unix containers
            if self.family == DockerFamily::Unix {
                capabilities.push(Version::CAP_FS_PERM.to_string());
            }

            let mut server_version: semver::Version = env!("CARGO_PKG_VERSION")
                .parse()
                .map_err(|e| io::Error::other(format!("Failed to parse version: {}", e)))?;

            if server_version.build.is_empty() {
                server_version.build =
                    semver::BuildMetadata::new(env!("CARGO_PKG_NAME")).map_err(|e| {
                        io::Error::other(format!("Failed to set build metadata: {}", e))
                    })?;
            }

            Ok(Version {
                server_version,
                protocol_version: PROTOCOL_VERSION,
                capabilities,
            })
        }
    }

    fn read_file(
        &self,
        _ctx: Ctx,
        path: PathBuf,
    ) -> impl std::future::Future<Output = io::Result<Vec<u8>>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();
            utils::tar_read_file(&self.client, &self.container, &path_str).await
        }
    }

    fn read_file_text(
        &self,
        _ctx: Ctx,
        path: PathBuf,
    ) -> impl std::future::Future<Output = io::Result<String>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();
            let data = utils::tar_read_file(&self.client, &self.container, &path_str).await?;
            String::from_utf8(data).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("File is not valid UTF-8: {}", e),
                )
            })
        }
    }

    fn write_file(
        &self,
        _ctx: Ctx,
        path: PathBuf,
        data: Vec<u8>,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();
            utils::tar_write_file(&self.client, &self.container, &path_str, &data).await
        }
    }

    fn write_file_text(
        &self,
        _ctx: Ctx,
        path: PathBuf,
        data: String,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();
            utils::tar_write_file(&self.client, &self.container, &path_str, data.as_bytes()).await
        }
    }

    fn append_file(
        &self,
        _ctx: Ctx,
        path: PathBuf,
        data: Vec<u8>,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();

            // Primary: try exec-based append
            match self.family {
                DockerFamily::Unix => {
                    let result = utils::execute_with_stdin(
                        &self.client,
                        &self.container,
                        &["sh", "-c", &format!("cat >> '{}'", path_str)],
                        &data,
                        self.user(),
                    )
                    .await;

                    if let Ok(output) = result
                        && output.success()
                    {
                        return Ok(());
                    }
                }
                DockerFamily::Windows => {
                    // Windows doesn't have a good stdin append; fall through to tar
                }
            }

            // Fallback: tar-read, append in memory, tar-write back
            let existing = utils::tar_read_file(&self.client, &self.container, &path_str)
                .await
                .unwrap_or_default();
            let mut combined = existing;
            combined.extend_from_slice(&data);
            utils::tar_write_file(&self.client, &self.container, &path_str, &combined).await
        }
    }

    fn append_file_text(
        &self,
        _ctx: Ctx,
        path: PathBuf,
        data: String,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();

            // Primary: try exec-based append
            match self.family {
                DockerFamily::Unix => {
                    let result = utils::execute_with_stdin(
                        &self.client,
                        &self.container,
                        &["sh", "-c", &format!("cat >> '{}'", path_str)],
                        data.as_bytes(),
                        self.user(),
                    )
                    .await;

                    if let Ok(output) = result
                        && output.success()
                    {
                        return Ok(());
                    }
                }
                DockerFamily::Windows => {}
            }

            // Fallback: tar-read, append in memory, tar-write back
            let existing = utils::tar_read_file(&self.client, &self.container, &path_str)
                .await
                .unwrap_or_default();
            let mut combined = String::from_utf8_lossy(&existing).to_string();
            combined.push_str(&data);
            utils::tar_write_file(
                &self.client,
                &self.container,
                &path_str,
                combined.as_bytes(),
            )
            .await
        }
    }

    fn read_dir(
        &self,
        _ctx: Ctx,
        path: PathBuf,
        depth: usize,
        absolute: bool,
        _canonicalize: bool,
        include_root: bool,
    ) -> impl std::future::Future<Output = io::Result<(Vec<DirEntry>, Vec<io::Error>)>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();
            let mut entries = Vec::new();
            let mut errors: Vec<io::Error> = Vec::new();

            // Try exec-based listing first for richer output
            let cmd = match self.family {
                DockerFamily::Unix => {
                    if depth == 0 || depth > 1 {
                        format!("find '{}' -printf '%y %p\\n'", path_str)
                    } else {
                        format!("find '{}' -maxdepth 1 -printf '%y %p\\n'", path_str)
                    }
                }
                DockerFamily::Windows => {
                    format!("dir /b /a \"{}\"", path_str)
                }
            };

            match self.run_shell_cmd(&cmd).await {
                Ok(output) if output.success() => {
                    let stdout = output.stdout_str();
                    for line in stdout.lines() {
                        if line.is_empty() {
                            continue;
                        }

                        let (file_type, entry_path) = if self.family == DockerFamily::Unix {
                            // Format: "type_char path"
                            let mut parts = line.splitn(2, ' ');
                            let type_char = parts.next().unwrap_or("f");
                            let p = parts.next().unwrap_or("");
                            if p.is_empty() {
                                continue;
                            }
                            let ft = match type_char {
                                "d" => FileType::Dir,
                                "l" => FileType::Symlink,
                                _ => FileType::File,
                            };
                            (ft, PathBuf::from(p))
                        } else {
                            // Windows: just filenames
                            let full_path = PathBuf::from(&path_str).join(line.trim());
                            (FileType::File, full_path)
                        };

                        // Calculate depth relative to root path
                        let rel_depth = entry_path
                            .strip_prefix(&path)
                            .map(|p| p.components().count())
                            .unwrap_or(0);

                        // Skip root entry unless requested
                        if rel_depth == 0 && !include_root {
                            continue;
                        }

                        // Apply depth filter
                        if depth > 0 && rel_depth > depth {
                            continue;
                        }

                        let display_path = if absolute {
                            entry_path.clone()
                        } else {
                            entry_path
                                .strip_prefix(&path)
                                .unwrap_or(&entry_path)
                                .to_path_buf()
                        };

                        entries.push(DirEntry {
                            path: display_path,
                            file_type,
                            depth: rel_depth,
                        });
                    }
                }
                _ => {
                    // Fallback to tar-based listing
                    match utils::tar_list_dir(&self.client, &self.container, &path_str).await {
                        Ok(tar_entries) => {
                            for (entry_type, entry_path, _size, _mtime) in tar_entries {
                                let file_type = match entry_type {
                                    tar::EntryType::Directory => FileType::Dir,
                                    tar::EntryType::Symlink | tar::EntryType::Link => {
                                        FileType::Symlink
                                    }
                                    _ => FileType::File,
                                };

                                let full_path = PathBuf::from(&entry_path);
                                let rel_depth = full_path.components().count().saturating_sub(1);

                                if rel_depth == 0 && !include_root {
                                    continue;
                                }

                                if depth > 0 && rel_depth > depth {
                                    continue;
                                }

                                entries.push(DirEntry {
                                    path: full_path,
                                    file_type,
                                    depth: rel_depth,
                                });
                            }
                        }
                        Err(e) => errors.push(e),
                    }
                }
            }

            Ok((entries, errors))
        }
    }

    fn create_dir(
        &self,
        _ctx: Ctx,
        path: PathBuf,
        all: bool,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();

            // Try exec-based mkdir first (faster, simpler)
            let cmd = match self.family {
                DockerFamily::Unix => {
                    if all {
                        format!("mkdir -p '{}'", path_str)
                    } else {
                        format!("mkdir '{}'", path_str)
                    }
                }
                DockerFamily::Windows => {
                    format!("mkdir \"{}\"", path_str)
                }
            };

            match self.run_shell_cmd(&cmd).await {
                Ok(output) if output.success() => Ok(()),
                _ => {
                    // Fallback to tar-based directory creation.
                    // The tar helpers include a zero-byte `.distant` marker file to force
                    // Docker to materialize directories on Windows nanoserver.
                    if all {
                        utils::tar_create_dir_all(&self.client, &self.container, &path_str).await?;
                    } else {
                        utils::tar_create_dir(&self.client, &self.container, &path_str).await?;
                    }

                    // Best-effort cleanup of the marker file.
                    // On Windows, use ContainerAdministrator because the marker is
                    // owned by SYSTEM (created via tar API) and ContainerUser lacks
                    // delete permissions.
                    let marker = match self.family {
                        DockerFamily::Unix => format!("{}/.distant", path_str),
                        DockerFamily::Windows => format!("{}\\.distant", path_str),
                    };
                    let del_cmd = match self.family {
                        DockerFamily::Unix => format!("rm -f '{}'", marker),
                        DockerFamily::Windows => format!("del /f \"{}\"", marker),
                    };
                    let _ = match self.family {
                        DockerFamily::Unix => self.run_shell_cmd(&del_cmd).await,
                        DockerFamily::Windows => {
                            self.run_shell_cmd_as("ContainerAdministrator", &del_cmd)
                                .await
                        }
                    };

                    Ok(())
                }
            }
        }
    }

    fn copy(
        &self,
        _ctx: Ctx,
        src: PathBuf,
        dst: PathBuf,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let src_str = src.to_string_lossy().to_string();
            let dst_str = dst.to_string_lossy().to_string();

            let cmd = match self.family {
                DockerFamily::Unix => format!("cp -r '{}' '{}'", src_str, dst_str),
                DockerFamily::Windows => format!("xcopy /E /I /Y \"{}\" \"{}\"", src_str, dst_str),
            };

            let output = self.run_shell_cmd_stdout(&cmd).await;
            match output {
                Ok(_) => Ok(()),
                Err(_) => {
                    // Fallback: tar-read src, tar-write to dst
                    let data =
                        utils::tar_read_file(&self.client, &self.container, &src_str).await?;
                    utils::tar_write_file(&self.client, &self.container, &dst_str, &data).await
                }
            }
        }
    }

    fn remove(
        &self,
        _ctx: Ctx,
        path: PathBuf,
        force: bool,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();

            if self.family == DockerFamily::Unix {
                let cmd = if force {
                    format!("rm -rf '{}'", path_str)
                } else {
                    format!("rm -r '{}'", path_str)
                };
                return self.run_shell_cmd_stdout(&cmd).await.map(|_| ());
            }

            // Windows: files/dirs created via Docker's tar API are owned by SYSTEM.
            // `ContainerUser` (nanoserver default) lacks NTFS delete permissions on
            // SYSTEM-owned entries. Use `ContainerAdministrator` for delete operations.
            // Try `rmdir /s /q` first (handles directories), then `del /f` for files,
            // as separate exec calls.
            let rmdir_cmd = format!("rmdir /s /q \"{}\"", path_str);
            if self
                .run_shell_cmd_as("ContainerAdministrator", &rmdir_cmd)
                .await
                .is_ok_and(|o| o.success())
            {
                return Ok(());
            }

            let del_cmd = format!("del /f /q \"{}\"", path_str);
            let output = self
                .run_shell_cmd_as("ContainerAdministrator", &del_cmd)
                .await?;
            if output.success() {
                Ok(())
            } else {
                Err(io::Error::other(format!(
                    "Command failed (exit {}): {}",
                    output.exit_code,
                    output.stderr_str()
                )))
            }
        }
    }

    fn rename(
        &self,
        _ctx: Ctx,
        src: PathBuf,
        dst: PathBuf,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let src_str = src.to_string_lossy().to_string();
            let dst_str = dst.to_string_lossy().to_string();

            let cmd = match self.family {
                DockerFamily::Unix => format!("mv '{}' '{}'", src_str, dst_str),
                DockerFamily::Windows => format!("move \"{}\" \"{}\"", src_str, dst_str),
            };

            match self.run_shell_cmd_stdout(&cmd).await {
                Ok(_) => Ok(()),
                Err(exec_err) => {
                    // Fallback: tar-read src → tar-write dst → exec-delete src.
                    // Only works for files; directory rename failures propagate.
                    let data =
                        match utils::tar_read_file(&self.client, &self.container, &src_str).await {
                            Ok(data) => data,
                            Err(_) => return Err(exec_err),
                        };

                    utils::tar_write_file(&self.client, &self.container, &dst_str, &data).await?;

                    // Delete the source file. On Windows, use ContainerAdministrator
                    // because SYSTEM-owned files (created via tar API) cannot be deleted
                    // by ContainerUser. Propagate errors — if the delete fails, the
                    // rename is incomplete.
                    let del_cmd = match self.family {
                        DockerFamily::Unix => format!("rm -f '{}'", src_str),
                        DockerFamily::Windows => format!("del /f \"{}\"", src_str),
                    };
                    let output = match self.family {
                        DockerFamily::Unix => self.run_shell_cmd(&del_cmd).await?,
                        DockerFamily::Windows => {
                            self.run_shell_cmd_as("ContainerAdministrator", &del_cmd)
                                .await?
                        }
                    };
                    if !output.success() {
                        return Err(io::Error::other(format!(
                            "Failed to delete source after rename (exit {}): {}",
                            output.exit_code,
                            output.stderr_str()
                        )));
                    }

                    Ok(())
                }
            }
        }
    }

    #[allow(unused_variables)]
    fn watch(
        &self,
        _ctx: Ctx,
        _path: PathBuf,
        _recursive: bool,
        _only: Vec<ChangeKind>,
        _except: Vec<ChangeKind>,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "File watching is not supported in Docker containers. \
                 No reliable filesystem event mechanism is available.",
            ))
        }
    }

    #[allow(unused_variables)]
    fn unwatch(
        &self,
        _ctx: Ctx,
        _path: PathBuf,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "File watching is not supported in Docker containers.",
            ))
        }
    }

    fn exists(
        &self,
        _ctx: Ctx,
        path: PathBuf,
    ) -> impl std::future::Future<Output = io::Result<bool>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();

            if self.family == DockerFamily::Windows {
                // On Windows nanoserver, the exec-based `if exist` check is unreliable:
                // it cannot see directories created via the Docker tar API (used as
                // fallback when `mkdir` fails due to ContainerUser permissions).
                // Use the tar-based check as the primary method — Docker's archive
                // API correctly reports existence regardless of how the path was created,
                // and returns 404 for truly deleted paths.
                return Ok(utils::tar_path_exists(&self.client, &self.container, &path_str).await);
            }

            // Unix: use exec-based check with tar fallback for infrastructure errors
            let cmd = format!("test -e '{}'", path_str);
            match self.run_shell_cmd(&cmd).await {
                Ok(output) => Ok(output.success()),
                Err(_) => {
                    Ok(utils::tar_path_exists(&self.client, &self.container, &path_str).await)
                }
            }
        }
    }

    fn metadata(
        &self,
        _ctx: Ctx,
        path: PathBuf,
        canonicalize: bool,
        _resolve_file_type: bool,
    ) -> impl std::future::Future<Output = io::Result<Metadata>> + Send {
        async move {
            let path_str = path.to_string_lossy().to_string();

            // Try exec-based stat first (Unix)
            if self.family == DockerFamily::Unix {
                let cmd = format!("stat -c '%F %s %Y %X %W %a %u %g %h %i' '{}'", path_str);
                if let Ok(output) = self.run_shell_cmd(&cmd).await
                    && output.success()
                {
                    let stdout = output.stdout_str();
                    if let Some(metadata) = parse_stat_output(stdout.trim(), &path, canonicalize) {
                        return Ok(metadata);
                    }
                }
            }

            // Fallback to tar-based metadata
            let entries = utils::tar_list_dir(&self.client, &self.container, &path_str).await?;

            if let Some((entry_type, _entry_path, size, mtime)) = entries.first() {
                let file_type = match entry_type {
                    tar::EntryType::Directory => FileType::Dir,
                    tar::EntryType::Symlink | tar::EntryType::Link => FileType::Symlink,
                    _ => FileType::File,
                };

                Ok(Metadata {
                    canonicalized_path: if canonicalize {
                        Some(path.clone())
                    } else {
                        None
                    },
                    file_type,
                    len: *size,
                    readonly: false,
                    accessed: None,
                    created: None,
                    modified: Some(*mtime),
                    unix: None,
                    windows: None,
                })
            } else {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Path not found: {}", path_str),
                ))
            }
        }
    }

    fn set_permissions(
        &self,
        _ctx: Ctx,
        path: PathBuf,
        permissions: Permissions,
        options: SetPermissionsOptions,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            if self.family == DockerFamily::Windows {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Permission setting is not fully supported on Windows containers",
                ));
            }

            let path_str = path.to_string_lossy().to_string();
            let mode = permissions.to_unix_mode();
            let mode_str = format!("{:o}", mode);

            let cmd = if options.recursive {
                format!("chmod -R {} '{}'", mode_str, path_str)
            } else {
                format!("chmod {} '{}'", mode_str, path_str)
            };

            self.run_shell_cmd_stdout(&cmd).await.map(|_| ())
        }
    }

    fn search(
        &self,
        ctx: Ctx,
        query: SearchQuery,
    ) -> impl std::future::Future<Output = io::Result<SearchId>> + Send {
        async move {
            let search_id: SearchId = rand::random();

            // Try exec-based search first if tools are available
            let mut matches = Vec::new();
            if self.search_tools.has_any()
                && let Ok(cmd) =
                    search::build_search_command(&query, &self.search_tools, self.family)
                && let Ok(output) = self.run_shell_cmd(&cmd).await
            {
                let stdout = output.stdout_str();
                matches = match query.target {
                    SearchQueryTarget::Contents => search::parse_contents_matches(&stdout),
                    SearchQueryTarget::Path => search::parse_path_matches(&stdout),
                };
            }

            // On Windows nanoserver, exec-based search tools (findstr, dir) cannot
            // access paths created via the Docker tar API. Fall back to a tar-based
            // search that reads files through the Docker archive API.
            if matches.is_empty() && self.family == DockerFamily::Windows {
                matches = self.tar_based_search(&query).await;
            }

            // Send results via reply
            use distant_core::protocol::Response;
            if !matches.is_empty() {
                let _ = ctx.reply.send(Response::SearchResults {
                    id: search_id,
                    matches,
                });
            }

            // Send done
            let _ = ctx.reply.send(Response::SearchDone { id: search_id });

            Ok(search_id)
        }
    }

    #[allow(unused_variables)]
    fn cancel_search(
        &self,
        _ctx: Ctx,
        id: SearchId,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            // Search runs synchronously in our implementation, so cancel is a no-op
            Ok(())
        }
    }

    fn proc_spawn(
        &self,
        ctx: Ctx,
        cmd: String,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
    ) -> impl std::future::Future<Output = io::Result<ProcessId>> + Send {
        let client = &self.client;
        let container = &self.container;
        let family = self.family;
        let user = self.user().map(|s| s.to_string());
        let processes = &self.processes;
        let global_processes = Arc::downgrade(processes);

        async move {
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
                exec_id,
            } = match pty {
                None => {
                    process::spawn_simple(
                        client,
                        container,
                        &cmd,
                        environment,
                        current_dir,
                        family,
                        user.as_deref(),
                        ctx.reply.clone_reply(),
                        make_cleanup(global_processes),
                    )
                    .await?
                }
                Some(size) => {
                    process::spawn_pty(
                        client,
                        container,
                        &cmd,
                        environment,
                        current_dir,
                        size,
                        family,
                        user.as_deref(),
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
                exec_id,
            };
            processes.write().await.insert(id, process);

            Ok(id)
        }
    }

    fn proc_kill(
        &self,
        _ctx: Ctx,
        id: ProcessId,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let mut processes = self.processes.write().await;
            match processes.get_mut(&id) {
                Some(process) => {
                    // Take the kill channel to send the signal. We do NOT remove the
                    // map entry here — the reader task's cleanup closure is the sole
                    // owner of removal. Removing here creates a race: if a new process
                    // reuses the same ProcessId before the reader finishes, the stale
                    // cleanup would delete the new entry.
                    if let Some(kill_tx) = process.kill_tx.take() {
                        let _ = kill_tx.send(()).await;
                        Ok(())
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("Process {} has already been killed", id),
                        ))
                    }
                }
                None => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("No process found with id {}", id),
                )),
            }
        }
    }

    fn proc_stdin(
        &self,
        _ctx: Ctx,
        id: ProcessId,
        data: Vec<u8>,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let processes = self.processes.read().await;
            match processes.get(&id) {
                Some(process) => {
                    if let Some(stdin_tx) = &process.stdin_tx {
                        stdin_tx.send(data).await.map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::BrokenPipe,
                                "Process stdin channel closed",
                            )
                        })
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "Process stdin is not available",
                        ))
                    }
                }
                None => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("No process found with id {}", id),
                )),
            }
        }
    }

    fn proc_resize_pty(
        &self,
        _ctx: Ctx,
        id: ProcessId,
        size: PtySize,
    ) -> impl std::future::Future<Output = io::Result<()>> + Send {
        async move {
            let processes = self.processes.read().await;
            match processes.get(&id) {
                Some(process) => {
                    if let Some(resize_tx) = &process.resize_tx {
                        resize_tx.send(size).await.map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::BrokenPipe,
                                "Process resize channel closed",
                            )
                        })
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "Process was not started with PTY support",
                        ))
                    }
                }
                None => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("No process found with id {}", id),
                )),
            }
        }
    }

    fn system_info(
        &self,
        _ctx: Ctx,
    ) -> impl std::future::Future<Output = io::Result<SystemInfo>> + Send {
        async move {
            let current_dir = self
                .cached_current_dir
                .get_or_try_init(async {
                    // Get working dir from container inspect or exec pwd
                    if let Some(wd) = &self.opts.working_dir {
                        return Ok::<PathBuf, io::Error>(PathBuf::from(wd));
                    }

                    match self.family {
                        DockerFamily::Unix => {
                            let output = self.run_cmd_stdout(&["pwd"]).await?;
                            Ok(PathBuf::from(output.trim()))
                        }
                        DockerFamily::Windows => {
                            let output = self.run_cmd_stdout(&["cmd", "/c", "cd"]).await?;
                            Ok(PathBuf::from(output.trim()))
                        }
                    }
                })
                .await?
                .clone();

            let username = self
                .cached_username
                .get_or_try_init(async {
                    match self.run_cmd_stdout(&["whoami"]).await {
                        Ok(output) => Ok::<String, io::Error>(output.trim().to_string()),
                        Err(_) => Ok::<String, io::Error>(String::from("unknown")),
                    }
                })
                .await?
                .clone();

            let shell = self
                .cached_shell
                .get_or_try_init(async {
                    match self.family {
                        DockerFamily::Unix => {
                            match self.run_shell_cmd_stdout("echo $SHELL").await {
                                Ok(output) => {
                                    let s = output.trim().to_string();
                                    if s.is_empty() {
                                        Ok::<String, io::Error>(String::from("/bin/sh"))
                                    } else {
                                        Ok(s)
                                    }
                                }
                                Err(_) => Ok(String::from("/bin/sh")),
                            }
                        }
                        DockerFamily::Windows => Ok(String::from("cmd.exe")),
                    }
                })
                .await?
                .clone();

            let (family, os, main_separator) = match self.family {
                DockerFamily::Unix => ("unix".to_string(), "linux".to_string(), '/'),
                DockerFamily::Windows => ("windows".to_string(), "windows".to_string(), '\\'),
            };

            // Try to get architecture
            let arch = match self.family {
                DockerFamily::Unix => self
                    .run_cmd_stdout(&["uname", "-m"])
                    .await
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| "unknown".to_string()),
                DockerFamily::Windows => "x86_64".to_string(),
            };

            Ok(SystemInfo {
                family,
                os,
                arch,
                current_dir,
                main_separator,
                username,
                shell,
            })
        }
    }
}

/// Parse Unix `stat` output into [`Metadata`].
///
/// Expected format: `%F %s %Y %X %W %a %u %g %h %i`
/// Example: `regular file 1234 1700000000 1700000000 1699000000 644 1000 1000 1 12345`
fn parse_stat_output(line: &str, path: &Path, canonicalize: bool) -> Option<Metadata> {
    let parts: Vec<&str> = line.splitn(10, ' ').collect();
    if parts.len() < 6 {
        return None;
    }

    // File type is the first N words before the size
    // "regular file", "directory", "symbolic link", etc.
    // We need to find where the numeric fields start
    let mut numeric_start = 0;
    for (i, part) in parts.iter().enumerate() {
        if part.parse::<u64>().is_ok() {
            numeric_start = i;
            break;
        }
    }

    if numeric_start == 0 {
        return None;
    }

    let type_str = parts[..numeric_start].join(" ");
    let file_type = match type_str.as_str() {
        "directory" => FileType::Dir,
        "symbolic link" => FileType::Symlink,
        _ => FileType::File,
    };

    let size = parts.get(numeric_start)?.parse::<u64>().ok()?;
    let modified = parts.get(numeric_start + 1)?.parse::<u64>().ok();
    let accessed = parts.get(numeric_start + 2)?.parse::<u64>().ok();
    let created_raw = parts.get(numeric_start + 3)?.parse::<u64>().ok();
    let created = created_raw.filter(|&v| v > 0); // stat returns 0 for unsupported
    let mode_str = parts.get(numeric_start + 4)?;
    let mode = u32::from_str_radix(mode_str, 8).ok()?;

    let readonly = mode & 0o200 == 0; // No write permission for owner

    let unix = Some(UnixMetadata {
        owner_read: mode & 0o400 != 0,
        owner_write: mode & 0o200 != 0,
        owner_exec: mode & 0o100 != 0,
        group_read: mode & 0o040 != 0,
        group_write: mode & 0o020 != 0,
        group_exec: mode & 0o010 != 0,
        other_read: mode & 0o004 != 0,
        other_write: mode & 0o002 != 0,
        other_exec: mode & 0o001 != 0,
    });

    Some(Metadata {
        canonicalized_path: if canonicalize {
            Some(path.to_path_buf())
        } else {
            None
        },
        file_type,
        len: size,
        readonly,
        accessed,
        created,
        modified,
        unix,
        windows: None,
    })
}
