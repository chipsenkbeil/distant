//! Utility functions for Docker exec operations and tar-based file I/O.

use std::io::{self, Read};
use std::path::Path;

use crate::DockerFamily;
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::query_parameters::{
    DownloadFromContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use bytes::Bytes;
use futures::StreamExt;
use log::*;

/// Output from executing a command in a container.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    /// Standard output bytes.
    pub stdout: Vec<u8>,

    /// Standard error bytes.
    pub stderr: Vec<u8>,

    /// Exit code (0 for success).
    pub exit_code: i64,
}

impl ExecOutput {
    /// Returns true if the command exited successfully (exit code 0).
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }

    /// Returns stdout as a UTF-8 string, lossy.
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    /// Returns stderr as a UTF-8 string, lossy.
    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}

/// Execute a command in a container and collect its output.
///
/// Returns the combined stdout, stderr, and exit code.
///
/// # Arguments
///
/// * `client` - Docker client handle
/// * `container` - Container name or ID
/// * `cmd` - Command and arguments to execute
/// * `user` - Optional user to run as
pub async fn execute_output(
    client: &Docker,
    container: &str,
    cmd: &[&str],
    user: Option<&str>,
) -> io::Result<ExecOutput> {
    let exec = client
        .create_exec(
            container,
            CreateExecOptions {
                cmd: Some(cmd.iter().map(|s| s.to_string()).collect()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                user: user.map(|u| u.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to create exec: {}", e)))?;

    let start_result = client
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: false,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to start exec: {}", e)))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    match start_result {
        StartExecResults::Attached { mut output, .. } => {
            while let Some(msg) = output.next().await {
                match msg {
                    Ok(bollard::container::LogOutput::StdOut { message }) => {
                        stdout.extend_from_slice(&message);
                    }
                    Ok(bollard::container::LogOutput::StdErr { message }) => {
                        stderr.extend_from_slice(&message);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return Err(io::Error::other(format!(
                            "Error reading exec output: {}",
                            e
                        )));
                    }
                }
            }
        }
        StartExecResults::Detached => {
            return Err(io::Error::other(
                "Exec started in detached mode unexpectedly",
            ));
        }
    }

    // Get the exit code
    let inspect = client
        .inspect_exec(&exec.id)
        .await
        .map_err(|e| io::Error::other(format!("Failed to inspect exec: {}", e)))?;

    let exit_code = inspect.exit_code.unwrap_or(-1);

    Ok(ExecOutput {
        stdout,
        stderr,
        exit_code,
    })
}

/// Execute a command in a container with stdin data.
pub async fn execute_with_stdin(
    client: &Docker,
    container: &str,
    cmd: &[&str],
    stdin_data: &[u8],
    user: Option<&str>,
) -> io::Result<ExecOutput> {
    use tokio::io::AsyncWriteExt;

    let exec = client
        .create_exec(
            container,
            CreateExecOptions {
                cmd: Some(cmd.iter().map(|s| s.to_string()).collect()),
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                user: user.map(|u| u.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to create exec: {}", e)))?;

    let start_result = client
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: false,
                ..Default::default()
            }),
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to start exec: {}", e)))?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    match start_result {
        StartExecResults::Attached {
            mut output,
            mut input,
        } => {
            // Write stdin data
            input
                .write_all(stdin_data)
                .await
                .map_err(|e| io::Error::other(format!("Failed to write stdin: {}", e)))?;
            input
                .shutdown()
                .await
                .map_err(|e| io::Error::other(format!("Failed to close stdin: {}", e)))?;

            // Read output
            while let Some(msg) = output.next().await {
                match msg {
                    Ok(bollard::container::LogOutput::StdOut { message }) => {
                        stdout.extend_from_slice(&message);
                    }
                    Ok(bollard::container::LogOutput::StdErr { message }) => {
                        stderr.extend_from_slice(&message);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return Err(io::Error::other(format!(
                            "Error reading exec output: {}",
                            e
                        )));
                    }
                }
            }
        }
        StartExecResults::Detached => {
            return Err(io::Error::other(
                "Exec started in detached mode unexpectedly",
            ));
        }
    }

    let inspect = client
        .inspect_exec(&exec.id)
        .await
        .map_err(|e| io::Error::other(format!("Failed to inspect exec: {}", e)))?;

    let exit_code = inspect.exit_code.unwrap_or(-1);

    Ok(ExecOutput {
        stdout,
        stderr,
        exit_code,
    })
}

/// Read a file from a container using the Docker archive (tar) API.
///
/// This works even in containers without a shell (distroless/scratch).
pub async fn tar_read_file(client: &Docker, container: &str, path: &str) -> io::Result<Vec<u8>> {
    let options = DownloadFromContainerOptionsBuilder::default()
        .path(path)
        .build();

    let mut stream = client.download_from_container(container, Some(options));
    let mut tar_data = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|e| io::Error::other(format!("Failed to download from container: {}", e)))?;
        tar_data.extend_from_slice(&chunk);
    }

    // Unpack the tar archive to get the file contents
    let mut archive = tar::Archive::new(&tar_data[..]);
    for entry in archive
        .entries()
        .map_err(|e| io::Error::other(format!("Failed to read tar: {}", e)))?
    {
        let mut entry =
            entry.map_err(|e| io::Error::other(format!("Failed to read tar entry: {}", e)))?;

        // Skip directory entries
        if entry.header().entry_type() == tar::EntryType::Directory {
            continue;
        }

        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .map_err(|e| io::Error::other(format!("Failed to read entry data: {}", e)))?;
        return Ok(contents);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("File not found in container: {}", path),
    ))
}

/// Write a file to a container using the Docker archive (tar) API.
///
/// This works even in containers without a shell (distroless/scratch).
pub async fn tar_write_file(
    client: &Docker,
    container: &str,
    path: &str,
    data: &[u8],
) -> io::Result<()> {
    let path_obj = Path::new(path);
    let parent = path_obj
        .parent()
        .unwrap_or(Path::new("/"))
        .to_string_lossy()
        .to_string();
    let filename = path_obj
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Path has no filename"))?
        .to_string_lossy()
        .to_string();

    let tar_data = build_tar_with_file(&filename, data)?;

    let options = UploadToContainerOptionsBuilder::default()
        .path(&parent)
        .build();

    client
        .upload_to_container(
            container,
            Some(options),
            bollard::body_full(Bytes::from(tar_data)),
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to upload to container: {}", e)))
}

/// Build a tar archive containing a single file.
fn build_tar_with_file(filename: &str, data: &[u8]) -> io::Result<Vec<u8>> {
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        let mut header = tar::Header::new_gnu();
        header.set_path(filename)?;
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
        header.set_cksum();
        builder.append(&header, data)?;
        builder.finish()?;
    }
    Ok(tar_buf)
}

/// Build a tar archive containing a directory entry and a zero-byte marker file.
///
/// The Docker `PUT /containers/{id}/archive` API silently accepts tar archives containing
/// only directory entries on Windows nanoserver but never materializes those directories.
/// Including a zero-byte `.distant` marker file forces Docker to create the directory.
pub fn build_tar_with_dir(dirname: &str) -> io::Result<Vec<u8>> {
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        let mtime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let dir_path = if dirname.ends_with('/') {
            dirname.to_string()
        } else {
            format!("{}/", dirname)
        };

        let mut header = tar::Header::new_gnu();
        header.set_path(&dir_path)?;
        header.set_size(0);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_mtime(mtime);
        header.set_cksum();
        builder.append(&header, &[] as &[u8])?;

        // Add a zero-byte marker file to force Docker to materialize the directory
        // on Windows nanoserver (directory-only tars are silently ignored).
        let marker_path = format!("{}.distant", dir_path);
        let mut marker_header = tar::Header::new_gnu();
        marker_header.set_path(&marker_path)?;
        marker_header.set_size(0);
        marker_header.set_mode(0o644);
        marker_header.set_entry_type(tar::EntryType::Regular);
        marker_header.set_mtime(mtime);
        marker_header.set_cksum();
        builder.append(&marker_header, &[] as &[u8])?;

        builder.finish()?;
    }
    Ok(tar_buf)
}

/// List entries in a directory by downloading it as a tar archive.
///
/// Returns a list of (entry_type, path, size, mtime) tuples.
pub async fn tar_list_dir(
    client: &Docker,
    container: &str,
    path: &str,
) -> io::Result<Vec<(tar::EntryType, String, u64, u64)>> {
    let options = DownloadFromContainerOptionsBuilder::default()
        .path(path)
        .build();

    let mut stream = client.download_from_container(container, Some(options));
    let mut tar_data = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            io::Error::other(format!(
                "Failed to download directory from container: {}",
                e
            ))
        })?;
        tar_data.extend_from_slice(&chunk);
    }

    let mut entries = Vec::new();
    let mut archive = tar::Archive::new(&tar_data[..]);

    for entry in archive
        .entries()
        .map_err(|e| io::Error::other(format!("Failed to read tar: {}", e)))?
    {
        let entry =
            entry.map_err(|e| io::Error::other(format!("Failed to read tar entry: {}", e)))?;

        let entry_type = entry.header().entry_type();
        let entry_path = entry
            .path()
            .map_err(|e| io::Error::other(format!("Failed to read entry path: {}", e)))?
            .to_string_lossy()
            .to_string();
        let size = entry.header().size().unwrap_or(0);
        let mtime = entry.header().mtime().unwrap_or(0);

        entries.push((entry_type, entry_path, size, mtime));
    }

    Ok(entries)
}

/// Check if a path exists in a container using the tar download API.
///
/// Returns true if the download succeeds, false on 404-style errors.
pub async fn tar_path_exists(client: &Docker, container: &str, path: &str) -> bool {
    let options = DownloadFromContainerOptionsBuilder::default()
        .path(path)
        .build();
    let mut stream = client.download_from_container(container, Some(options));

    // Just check if we can get the first chunk
    matches!(stream.next().await, Some(Ok(_)))
}

/// Available search tools detected in a container.
#[derive(Debug, Clone, Default)]
pub struct SearchTools {
    /// Whether ripgrep is available.
    pub has_rg: bool,

    /// Whether GNU grep is available.
    pub has_grep: bool,

    /// Whether find is available.
    pub has_find: bool,

    /// Whether Windows `findstr.exe` is available.
    pub has_findstr: bool,
}

impl SearchTools {
    /// Returns true if at least basic search capability is available.
    pub fn has_any(&self) -> bool {
        self.has_rg || self.has_find || self.has_findstr
    }
}

/// Probe the container for available search tools.
///
/// Uses `which` on Unix containers and direct invocation on Windows containers
/// (nanoserver lacks `which` and `where.exe`).
pub async fn probe_search_tools(
    client: &Docker,
    container: &str,
    family: DockerFamily,
) -> SearchTools {
    let mut tools = SearchTools::default();

    match family {
        DockerFamily::Unix => {
            // Check for ripgrep
            if let Ok(output) = execute_output(client, container, &["which", "rg"], None).await {
                tools.has_rg = output.success();
            }

            // Check for grep
            if let Ok(output) = execute_output(client, container, &["which", "grep"], None).await {
                tools.has_grep = output.success();
            }

            // Check for find
            if let Ok(output) = execute_output(client, container, &["which", "find"], None).await {
                tools.has_find = output.success();
            }
        }
        DockerFamily::Windows => {
            // Nanoserver lacks `which` and `where.exe`, so probe by direct invocation.
            if let Ok(output) = execute_output(client, container, &["findstr", "/?"], None).await {
                tools.has_findstr = output.success();
            }
        }
    }

    debug!(
        "Search tools: rg={}, grep={}, find={}, findstr={}",
        tools.has_rg, tools.has_grep, tools.has_find, tools.has_findstr
    );

    tools
}

/// Create a directory in a container using the tar upload API (fallback, no exec needed).
///
/// Uploads a tar archive containing the directory entry to the parent path.
/// For nested directory creation, use [`tar_create_dir_all`] instead.
pub async fn tar_create_dir(client: &Docker, container: &str, path: &str) -> io::Result<()> {
    let path_obj = Path::new(path);
    let parent = path_obj
        .parent()
        .unwrap_or(Path::new("/"))
        .to_string_lossy()
        .to_string();
    let dirname = path_obj
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Path has no directory name"))?
        .to_string_lossy()
        .to_string();

    let tar_data = build_tar_with_dir(&dirname)?;

    let options = UploadToContainerOptionsBuilder::default()
        .path(&parent)
        .build();

    client
        .upload_to_container(
            container,
            Some(options),
            bollard::body_full(Bytes::from(tar_data)),
        )
        .await
        .map_err(|e| io::Error::other(format!("Failed to create directory in container: {}", e)))
}

/// Create a directory and all ancestor directories using the tar upload API.
///
/// Builds a single tar archive containing directory entries for each path component
/// relative to the root, then uploads it to the filesystem root. This is the tar-based
/// equivalent of `mkdir -p`.
pub async fn tar_create_dir_all(client: &Docker, container: &str, path: &str) -> io::Result<()> {
    let path_obj = Path::new(path);

    // Determine the filesystem root and the relative components to create.
    // On Windows paths like C:\foo\bar, the root is C:\ and components are [foo, bar].
    // On Unix paths like /foo/bar, the root is / and components are [foo, bar].
    let mut components = Vec::new();
    let mut root = String::new();

    for component in path_obj.components() {
        match component {
            std::path::Component::Prefix(p) => {
                root = p.as_os_str().to_string_lossy().to_string();
            }
            std::path::Component::RootDir => {
                if root.is_empty() {
                    root = "/".to_string();
                } else {
                    root.push(std::path::MAIN_SEPARATOR);
                }
            }
            std::path::Component::Normal(c) => {
                components.push(c.to_string_lossy().to_string());
            }
            _ => {}
        }
    }

    if components.is_empty() {
        return Ok(());
    }

    if root.is_empty() {
        root = "/".to_string();
    }

    // Build a tar archive with directory entries for each cumulative path.
    // e.g. for components [a, b, c] → entries "a/", "a/b/", "a/b/c/"
    // A zero-byte marker file is added at the deepest directory to force
    // Docker to materialize the directories on Windows nanoserver.
    let mut tar_buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_buf);
        let mtime = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut cumulative = String::new();
        for component in &components {
            if !cumulative.is_empty() {
                cumulative.push('/');
            }
            cumulative.push_str(component);

            let dir_path = format!("{}/", cumulative);
            let mut header = tar::Header::new_gnu();
            header.set_path(&dir_path)?;
            header.set_size(0);
            header.set_mode(0o755);
            header.set_entry_type(tar::EntryType::Directory);
            header.set_mtime(mtime);
            header.set_cksum();
            builder.append(&header, &[] as &[u8])?;
        }

        // Add a zero-byte marker file at the deepest directory
        let marker_path = format!("{}/.distant", cumulative);
        let mut marker_header = tar::Header::new_gnu();
        marker_header.set_path(&marker_path)?;
        marker_header.set_size(0);
        marker_header.set_mode(0o644);
        marker_header.set_entry_type(tar::EntryType::Regular);
        marker_header.set_mtime(mtime);
        marker_header.set_cksum();
        builder.append(&marker_header, &[] as &[u8])?;

        builder.finish()?;
    }

    let options = UploadToContainerOptionsBuilder::default()
        .path(&root)
        .build();

    client
        .upload_to_container(
            container,
            Some(options),
            bollard::body_full(Bytes::from(tar_buf)),
        )
        .await
        .map_err(|e| {
            io::Error::other(format!(
                "Failed to create directory tree in container: {}",
                e
            ))
        })
}
