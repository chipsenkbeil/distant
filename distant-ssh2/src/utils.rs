use async_compat::CompatExt;
use std::{
    fmt, io,
    path::{Component, Path, PathBuf, Prefix},
    time::Duration,
};
use wezterm_ssh::{ExecResult, Session, Sftp};

#[allow(dead_code)]
const READER_PAUSE_MILLIS: u64 = 100;

#[derive(Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl fmt::Debug for ExecOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let is_alternate = f.alternate();

        let mut s = f.debug_struct("ExecOutput");
        s.field("success", &self.success);

        if is_alternate {
            s.field("stdout", &String::from_utf8_lossy(&self.stdout))
                .field("stderr", &String::from_utf8_lossy(&self.stderr));
        } else {
            s.field("stdout", &self.stdout)
                .field("stderr", &self.stderr);
        }

        s.finish()
    }
}

#[allow(dead_code)]
pub async fn execute_output(session: &Session, cmd: &str) -> io::Result<ExecOutput> {
    let ExecResult {
        mut child,
        mut stdout,
        mut stderr,
        ..
    } = session
        .exec(cmd, None)
        .compat()
        .await
        .map_err(to_other_error)?;

    macro_rules! spawn_reader {
        ($reader:ident) => {{
            $reader.set_non_blocking(true).map_err(to_other_error)?;
            tokio::spawn(async move {
                use std::io::Read;
                let mut bytes = Vec::new();
                let mut buf = [0u8; 1024];
                loop {
                    match $reader.read(&mut buf) {
                        Ok(n) if n > 0 => bytes.extend(&buf[..n]),
                        Ok(_) => break Ok(bytes),
                        Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                            tokio::time::sleep(Duration::from_millis(READER_PAUSE_MILLIS)).await;
                        }
                        Err(x) => break Err(x),
                    }
                }
            })
        }};
    }

    // Spawn async readers for stdout and stderr from process
    let stdout_handle = spawn_reader!(stdout);
    let stderr_handle = spawn_reader!(stderr);

    // Wait for our handles to conclude
    let stdout = stdout_handle.await.map_err(to_other_error)??;
    let stderr = stderr_handle.await.map_err(to_other_error)??;

    // Wait for process to conclude
    let status = child.async_wait().compat().await.map_err(to_other_error)?;

    Ok(ExecOutput {
        success: status.success(),
        stdout,
        stderr,
    })
}

/// Performs canonicalization of the given path using SFTP with various handling of Windows paths
pub async fn canonicalize(sftp: &Sftp, path: impl AsRef<Path>) -> io::Result<PathBuf> {
    // Determine if we are supplying a Windows path
    let mut is_windows_path = path
        .as_ref()
        .components()
        .any(|c| matches!(c, Component::Prefix(_)));

    // Try to canonicalize original path first
    let result = sftp
        .canonicalize(path.as_ref().to_path_buf())
        .compat()
        .await;

    // If we don't see the path initially as a Windows path, but we can find a drive letter after
    // canonicalization, still treat it as a windows path
    //
    // NOTE: This is for situations where we are given a relative path like '.' where we cannot
    //       infer the path is for Windows out of the box
    if !is_windows_path {
        if let Ok(path) = result.as_ref() {
            is_windows_path = drive_letter(path.as_std_path()).is_some();
        }
    }

    // If result is a failure, we want to try again with a unix path in case we were using
    // a windows path and sshd had a problem with canonicalizing it
    let unix_path = if result.is_err() && is_windows_path {
        Some(to_unix_path(path.as_ref()))
    } else {
        None
    };

    // 1. If we succeeded on first try, return that path
    //     a. If the canonicalized path was for a Windows path, sftp may return something odd
    //        like C:\Users\example -> /c:/Users/example and we need to transform it back
    //     b. Otherwise, if the input path was a unix path, we return canonicalized as is
    // 2. If we failed on first try and have a clear Windows path, try the unix version
    //    and then convert result back to windows version, return our original error if we fail
    // 3. If we failed and there is no valid unix path for a Windows path, return the
    //    original error
    match (result, unix_path) {
        (Ok(path), _) if is_windows_path => Ok(to_windows_path(path.as_std_path())),
        (Ok(path), _) => Ok(path.into_std_path_buf()),
        (Err(x), Some(path)) => Ok(to_windows_path(
            &sftp
                .canonicalize(path.to_path_buf())
                .compat()
                .await
                .map_err(|_| to_other_error(x))?
                .into_std_path_buf(),
        )),
        (Err(x), None) => Err(to_other_error(x)),
    }
}

/// Convert a path into unix-oriented path
///
/// E.g. C:\Users\example\Documents\file.txt -> /c/Users/example/Documents/file.txt
pub fn to_unix_path(path: &Path) -> PathBuf {
    let is_windows_path = path.components().any(|c| matches!(c, Component::Prefix(_)));

    if !is_windows_path {
        return path.to_path_buf();
    }

    let mut p = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(x) => match x.kind() {
                Prefix::Verbatim(path) => p.push(path),
                Prefix::VerbatimUNC(hostname, share) => {
                    p.push(hostname);
                    p.push(share);
                }
                Prefix::VerbatimDisk(letter) => {
                    p.push(format!("/{}", letter as char));
                }
                Prefix::DeviceNS(device_name) => p.push(device_name),
                Prefix::UNC(hostname, share) => {
                    p.push(hostname);
                    p.push(share);
                }
                Prefix::Disk(letter) => {
                    p.push(format!("/{}", letter as char));
                }
            },

            // If we have a prefix, then we are dropping it and converting into
            // a root and normal component, so we will now skip this root
            Component::RootDir => continue,

            x => p.push(x),
        }
    }

    p
}

/// Convert a path into windows-oriented path
///
/// E.g. /c/Users/example/Documents/file.txt -> C:\Users\example\Documents\file.txt
pub fn to_windows_path(path: &Path) -> PathBuf {
    let is_windows_path = path.components().any(|c| matches!(c, Component::Prefix(_)));

    if is_windows_path {
        return path.to_path_buf();
    }

    // See if we have a drive letter at the beginning, otherwise default to C:\
    let drive_letter = drive_letter(path);

    let mut p = PathBuf::new();

    // Start with a drive prefix
    p.push(format!("{}:", drive_letter.unwrap_or('C')));

    let mut components = path.components();

    // If we start with a root portion of the regular path, we want to drop
    // it and the drive letter since we've added that separately
    if path.has_root() {
        p.push(Component::RootDir);
        components.next();

        if drive_letter.is_some() {
            components.next();
        }
    }

    for component in components {
        p.push(component);
    }

    p
}

/// Looks for a drive letter in the given path
pub fn drive_letter(path: &Path) -> Option<char> {
    // See if we are a windows path, and if so grab the letter from the components
    let maybe_letter = path.components().find_map(|c| match c {
        Component::Prefix(x) => match x.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => Some(letter as char),
            _ => None,
        },
        _ => None,
    });

    if let Some(letter) = maybe_letter {
        return Some(letter);
    }

    // If there was no drive letter and we are not a root, there is nothing left to find
    if !path.has_root() {
        return None;
    }

    // Otherwise, scan just after root for a drive letter
    path.components().nth(1).and_then(|c| match c {
        Component::Normal(s) => s.to_str().and_then(|s| {
            let mut chars = s.chars();
            let first = chars.next();
            let second = chars.next();
            let has_more = chars.next().is_some();

            if has_more {
                return None;
            }

            match (first, second) {
                (letter, Some(':') | None) => letter,
                _ => None,
            }
        }),
        _ => None,
    })
}

/// Determines if using windows by checking the canonicalized path of '.'
pub async fn is_windows(sftp: &Sftp) -> io::Result<bool> {
    // Look up the current directory
    let current_dir = canonicalize(sftp, ".").await?;

    // TODO: Ideally, we would determine the family using something like the following:
    //
    //      cmd.exe /C echo %OS%
    //
    //      Determine OS by printing OS variable (works with Windows 2000+)
    //      If it matches Windows_NT, then we are on windows
    //
    // However, the above is not working for whatever reason (always has success == false); so,
    // we're purely using a check if we have a drive letter on the canonicalized path to
    // determine if on windows for now. Some sort of failure with SIGPIPE
    Ok(current_dir
        .components()
        .any(|c| matches!(c, Component::Prefix(_))))
}

pub fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::Other, err)
}
