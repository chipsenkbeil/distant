use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use distant_core::protocol::{FileType, RemotePath};
use distant_core::{Channel, ChannelExt};
use log::debug;

use crate::cli::common::Ui;

/// A parsed transfer path — either local or remote.
enum TransferPath {
    Local(PathBuf),
    Remote(String),
}

/// The resolved direction of a copy operation.
#[derive(Debug)]
enum TransferDirection {
    Upload { local: PathBuf, remote: String },
    Download { remote: String, local: PathBuf },
}

/// Parse src and dst strings into a transfer direction.
///
/// A leading `:` marks a path as remote. Exactly one of src/dst must be remote.
fn parse_transfer_paths(src: &str, dst: &str) -> anyhow::Result<TransferDirection> {
    let src_path = parse_single_path(src);
    let dst_path = parse_single_path(dst);

    match (src_path, dst_path) {
        (TransferPath::Local(local), TransferPath::Remote(remote)) => {
            if remote.is_empty() {
                bail!("Remote path is empty (did you mean `:/path`?)");
            }
            Ok(TransferDirection::Upload { local, remote })
        }
        (TransferPath::Remote(remote), TransferPath::Local(local)) => {
            if remote.is_empty() {
                bail!("Remote path is empty (did you mean `:/path`?)");
            }
            Ok(TransferDirection::Download { remote, local })
        }
        (TransferPath::Local(_), TransferPath::Local(_)) => {
            bail!(
                "Both paths are local. Use your system's cp command, or prefix a remote path with `:`"
            )
        }
        (TransferPath::Remote(_), TransferPath::Remote(_)) => {
            bail!("Both paths are remote. Use `distant fs copy` for remote-to-remote copies")
        }
    }
}

fn parse_single_path(s: &str) -> TransferPath {
    if let Some(stripped) = s.strip_prefix(':') {
        TransferPath::Remote(stripped.to_string())
    } else {
        TransferPath::Local(PathBuf::from(s))
    }
}

/// Format a byte count as a human-readable string.
fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if n >= TB {
        format!("{:.1} TB", n as f64 / TB as f64)
    } else if n >= GB {
        format!("{:.1} GB", n as f64 / GB as f64)
    } else if n >= MB {
        format!("{:.1} MB", n as f64 / MB as f64)
    } else if n >= KB {
        format!("{:.1} KB", n as f64 / KB as f64)
    } else {
        format!("{n} B")
    }
}

/// Entry point for `distant copy`.
pub async fn run_copy(
    channel: &mut Channel,
    src: &str,
    dst: &str,
    recursive: bool,
    ui: &Ui,
) -> anyhow::Result<()> {
    let direction = parse_transfer_paths(src, dst)?;

    match direction {
        TransferDirection::Upload { local, remote } => {
            let meta = tokio::fs::metadata(&local)
                .await
                .with_context(|| format!("Failed to read {}", local.display()))?;

            if meta.is_dir() {
                if !recursive {
                    bail!(
                        "{} is a directory (use -r to copy recursively)",
                        local.display()
                    );
                }
                upload_dir(channel, &local, &remote, ui).await
            } else {
                upload_file(channel, &local, &remote, ui).await
            }
        }
        TransferDirection::Download { remote, local } => {
            let meta = channel
                .metadata(RemotePath::new(&remote), false, true)
                .await
                .with_context(|| format!("Failed to read remote path {remote}"))?;

            if meta.file_type == FileType::Dir {
                if !recursive {
                    bail!("{remote} is a directory (use -r to copy recursively)");
                }
                download_dir(channel, &remote, &local, ui).await
            } else {
                download_file(channel, &remote, &local, meta.len, ui).await
            }
        }
    }
}

/// Resolve the final destination path for a file transfer.
///
/// Like cp/scp: if dst is an existing directory, place the source inside it
/// with its original filename. Otherwise treat dst as the target path.
async fn resolve_remote_dst(channel: &mut Channel, remote: &str, local_name: &str) -> String {
    // Check if remote path is an existing directory
    if let Ok(meta) = channel.metadata(RemotePath::new(remote), false, true).await
        && meta.file_type == FileType::Dir
    {
        // Place inside the directory
        let separator = if remote.contains('\\') { '\\' } else { '/' };
        return format!("{remote}{separator}{local_name}");
    }
    remote.to_string()
}

async fn resolve_local_dst(local: &Path, remote_name: &str) -> PathBuf {
    if let Ok(meta) = tokio::fs::metadata(local).await
        && meta.is_dir()
    {
        return local.join(remote_name);
    }
    local.to_path_buf()
}

/// Extract the last component of a path (works for both local and remote paths).
fn path_file_name(path: &str) -> &str {
    // Handle both / and \ separators
    let name = path
        .rsplit_once('/')
        .map(|(_, name)| name)
        .or_else(|| path.rsplit_once('\\').map(|(_, name)| name))
        .unwrap_or(path);
    if name.is_empty() { path } else { name }
}

async fn upload_file(
    channel: &mut Channel,
    local: &Path,
    remote: &str,
    ui: &Ui,
) -> anyhow::Result<()> {
    let local_name = local
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let remote = resolve_remote_dst(channel, remote, &local_name).await;

    let data = tokio::fs::read(local)
        .await
        .with_context(|| format!("Failed to read {}", local.display()))?;
    let size = data.len() as u64;
    let name = &local_name;

    debug!(
        "Uploading {} ({}) to {}",
        local.display(),
        format_bytes(size),
        remote
    );
    let sp = ui.spinner(&format!("Uploading {name} ({})...", format_bytes(size)));

    channel
        .write_file(RemotePath::new(&remote), data)
        .await
        .with_context(|| format!("Failed to write remote file {remote}"))?;

    sp.done(&format!("Uploaded {name} ({})", format_bytes(size)));
    Ok(())
}

async fn download_file(
    channel: &mut Channel,
    remote: &str,
    local: &Path,
    size: u64,
    ui: &Ui,
) -> anyhow::Result<()> {
    let remote_name = path_file_name(remote);
    let local = resolve_local_dst(local, remote_name).await;

    debug!(
        "Downloading {} ({}) to {}",
        remote,
        format_bytes(size),
        local.display()
    );
    let sp = ui.spinner(&format!(
        "Downloading {remote_name} ({})...",
        format_bytes(size)
    ));

    let data = channel
        .read_file(RemotePath::new(remote))
        .await
        .with_context(|| format!("Failed to read remote file {remote}"))?;

    // Create parent directories if needed
    if let Some(parent) = local.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    let actual_size = data.len() as u64;
    tokio::fs::write(&local, data)
        .await
        .with_context(|| format!("Failed to write {}", local.display()))?;

    sp.done(&format!(
        "Downloaded {remote_name} ({})",
        format_bytes(actual_size)
    ));
    Ok(())
}

/// Recursively walk a local directory, collecting (relative_path, is_dir) entries.
async fn walk_local_dir(base: &Path) -> anyhow::Result<Vec<(PathBuf, bool)>> {
    let mut entries = Vec::new();
    let mut stack = vec![PathBuf::new()];

    while let Some(rel_dir) = stack.pop() {
        let abs_dir = base.join(&rel_dir);
        let mut rd = tokio::fs::read_dir(&abs_dir)
            .await
            .with_context(|| format!("Failed to read directory {}", abs_dir.display()))?;

        while let Some(entry) = rd.next_entry().await? {
            let file_type = entry.file_type().await?;
            let name = entry.file_name();
            let rel_path = rel_dir.join(name);

            if file_type.is_dir() {
                entries.push((rel_path.clone(), true));
                stack.push(rel_path);
            } else {
                entries.push((rel_path, false));
            }
        }
    }

    // Sort for deterministic order
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

async fn upload_dir(
    channel: &mut Channel,
    local: &Path,
    remote: &str,
    ui: &Ui,
) -> anyhow::Result<()> {
    let local_name = local
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| local.display().to_string());

    let remote_base = resolve_remote_dst(channel, remote, &local_name).await;
    let separator = if remote_base.contains('\\') {
        "\\"
    } else {
        "/"
    };

    let entries = walk_local_dir(local).await?;
    let file_entries: Vec<_> = entries.iter().filter(|(_, is_dir)| !*is_dir).collect();
    let dir_entries: Vec<_> = entries.iter().filter(|(_, is_dir)| *is_dir).collect();
    let total_files = file_entries.len();

    debug!(
        "Uploading directory {} ({} files, {} subdirs) to {}",
        local.display(),
        total_files,
        dir_entries.len(),
        remote_base
    );
    let sp = ui.spinner(&format!("Uploading {local_name} ({total_files} files)..."));

    // Create the base remote directory
    channel
        .create_dir(RemotePath::new(&remote_base), true)
        .await
        .with_context(|| format!("Failed to create remote directory {remote_base}"))?;

    // Create subdirectories
    for (rel_path, _) in &dir_entries {
        let remote_dir = format!(
            "{remote_base}{separator}{}",
            rel_path.to_string_lossy().replace('\\', separator)
        );
        channel
            .create_dir(RemotePath::new(&remote_dir), true)
            .await
            .with_context(|| format!("Failed to create remote directory {remote_dir}"))?;
    }

    // Upload files
    let mut total_size: u64 = 0;
    for (i, (rel_path, _)) in file_entries.iter().enumerate() {
        let local_file = local.join(rel_path);
        let remote_file = format!(
            "{remote_base}{separator}{}",
            rel_path.to_string_lossy().replace('\\', separator)
        );

        let data = tokio::fs::read(&local_file)
            .await
            .with_context(|| format!("Failed to read {}", local_file.display()))?;
        total_size += data.len() as u64;

        channel
            .write_file(RemotePath::new(&remote_file), data)
            .await
            .with_context(|| format!("Failed to write remote file {remote_file}"))?;

        sp.set_message(format!(
            "Uploading {local_name} ({}/{total_files} files)...",
            i + 1
        ));
    }

    sp.done(&format!(
        "Uploaded {local_name} ({total_files} files, {})",
        format_bytes(total_size)
    ));
    Ok(())
}

async fn download_dir(
    channel: &mut Channel,
    remote: &str,
    local: &Path,
    ui: &Ui,
) -> anyhow::Result<()> {
    let remote_name = path_file_name(remote);
    let local_base = resolve_local_dst(local, remote_name).await;

    // Read the full remote directory listing (depth 0 = unlimited, relative paths, no root)
    let (dir_entries, failures) = channel
        .read_dir(RemotePath::new(remote), 0, false, false, false)
        .await
        .with_context(|| format!("Failed to list remote directory {remote}"))?;

    if !failures.is_empty() {
        debug!(
            "Remote directory listing had {} failures; some entries may be skipped",
            failures.len()
        );
    }

    let dirs: Vec<_> = dir_entries
        .iter()
        .filter(|e| e.file_type == FileType::Dir)
        .collect();
    let files: Vec<_> = dir_entries
        .iter()
        .filter(|e| e.file_type == FileType::File)
        .collect();
    let total_files = files.len();

    debug!(
        "Downloading directory {} ({} files, {} subdirs) to {}",
        remote,
        total_files,
        dirs.len(),
        local_base.display()
    );
    let sp = ui.spinner(&format!(
        "Downloading {remote_name} ({total_files} files)..."
    ));

    // Create local base directory
    tokio::fs::create_dir_all(&local_base)
        .await
        .with_context(|| format!("Failed to create directory {}", local_base.display()))?;

    // Determine separator for building remote paths
    let separator = if remote.contains('\\') { "\\" } else { "/" };

    // Create subdirectories
    for dir_entry in &dirs {
        let local_dir = local_base.join(dir_entry.path.as_str());
        tokio::fs::create_dir_all(&local_dir)
            .await
            .with_context(|| format!("Failed to create directory {}", local_dir.display()))?;
    }

    // Download files
    let mut total_size: u64 = 0;
    for (i, file_entry) in files.iter().enumerate() {
        let local_file = local_base.join(file_entry.path.as_str());

        // Ensure parent directory exists
        if let Some(parent) = local_file.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Build full remote path from base + relative entry path
        let remote_file_path = format!(
            "{remote}{separator}{}",
            file_entry.path.as_str().replace('\\', separator)
        );
        let data = channel
            .read_file(RemotePath::new(&remote_file_path))
            .await
            .with_context(|| format!("Failed to read remote file {}", file_entry.path))?;
        total_size += data.len() as u64;

        tokio::fs::write(&local_file, data)
            .await
            .with_context(|| format!("Failed to write {}", local_file.display()))?;

        sp.set_message(format!(
            "Downloading {remote_name} ({}/{total_files} files)...",
            i + 1
        ));
    }

    sp.done(&format!(
        "Downloaded {remote_name} ({total_files} files, {})",
        format_bytes(total_size)
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod parse_transfer_paths {
        use super::*;

        #[test]
        fn upload_local_to_remote() {
            let dir = parse_transfer_paths("./local.txt", ":/remote/file.txt").unwrap();
            match dir {
                TransferDirection::Upload { local, remote } => {
                    assert_eq!(local, PathBuf::from("./local.txt"));
                    assert_eq!(remote, "/remote/file.txt");
                }
                _ => panic!("Expected Upload"),
            }
        }

        #[test]
        fn download_remote_to_local() {
            let dir = parse_transfer_paths(":/remote/file.txt", "./local.txt").unwrap();
            match dir {
                TransferDirection::Download { remote, local } => {
                    assert_eq!(remote, "/remote/file.txt");
                    assert_eq!(local, PathBuf::from("./local.txt"));
                }
                _ => panic!("Expected Download"),
            }
        }

        #[test]
        fn both_local_is_error() {
            let err = parse_transfer_paths("./a", "./b").unwrap_err();
            assert!(
                err.to_string().contains("Both paths are local"),
                "Unexpected error: {err}"
            );
        }

        #[test]
        fn both_remote_is_error() {
            let err = parse_transfer_paths(":/a", ":/b").unwrap_err();
            assert!(
                err.to_string().contains("Both paths are remote"),
                "Unexpected error: {err}"
            );
        }

        #[test]
        fn colon_only_is_error() {
            let err = parse_transfer_paths("./file", ":").unwrap_err();
            assert!(
                err.to_string().contains("Remote path is empty"),
                "Unexpected error: {err}"
            );
        }

        #[test]
        fn remote_windows_path() {
            let dir = parse_transfer_paths(":C:\\Users\\test\\file.txt", "./local.txt").unwrap();
            match dir {
                TransferDirection::Download { remote, .. } => {
                    assert_eq!(remote, "C:\\Users\\test\\file.txt");
                }
                _ => panic!("Expected Download"),
            }
        }
    }

    mod format_bytes {
        use super::*;

        #[test]
        fn zero_bytes() {
            assert_eq!(format_bytes(0), "0 B");
        }

        #[test]
        fn bytes_below_kb() {
            assert_eq!(format_bytes(1023), "1023 B");
        }

        #[test]
        fn exactly_one_kb() {
            assert_eq!(format_bytes(1024), "1.0 KB");
        }

        #[test]
        fn megabytes() {
            assert_eq!(format_bytes(1_572_864), "1.5 MB");
        }

        #[test]
        fn gigabytes() {
            assert_eq!(format_bytes(2_147_483_648), "2.0 GB");
        }

        #[test]
        fn terabytes() {
            assert_eq!(format_bytes(1_099_511_627_776), "1.0 TB");
        }
    }

    mod path_file_name {
        use super::*;

        #[test]
        fn unix_path() {
            assert_eq!(path_file_name("/home/user/file.txt"), "file.txt");
        }

        #[test]
        fn windows_path() {
            assert_eq!(path_file_name("C:\\Users\\test\\file.txt"), "file.txt");
        }

        #[test]
        fn bare_name() {
            assert_eq!(path_file_name("file.txt"), "file.txt");
        }

        #[test]
        fn trailing_slash_returns_whole_path() {
            // trailing slash means empty last component, so we return the whole path
            assert_eq!(path_file_name("/home/user/"), "/home/user/");
        }
    }
}
