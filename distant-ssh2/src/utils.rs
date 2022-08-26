use async_compat::CompatExt;
use std::{
    fmt, io,
    path::{Path, PathBuf},
    time::Duration,
};
use typed_path::{windows::WindowsComponent, Components, WindowsPath, WindowsPathBuf};
use wezterm_ssh::{ExecResult, Session, Sftp};

const SSH_EXEC_TIMEOUT: Option<Duration> = Some(Duration::from_secs(1));

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
pub async fn execute_output(
    session: &Session,
    cmd: &str,
    timeout: Option<Duration>,
) -> io::Result<ExecOutput> {
    let ExecResult {
        mut child,
        mut stdout,
        mut stderr,
        stdin: _stdin,
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

    // Wait for process to conclude
    let status = child.async_wait().compat().await.map_err(to_other_error)?;

    // Wait for our handles to conclude (max of timeout if provided)
    let (stdout, stderr) = match timeout {
        Some(duration) => {
            let (res1, res2) = tokio::try_join!(
                tokio::time::timeout(duration, stdout_handle),
                tokio::time::timeout(duration, stderr_handle)
            )?;
            (res1??, res2??)
        }
        None => {
            let (res1, res2) = tokio::try_join!(stdout_handle, stderr_handle)?;
            (res1?, res2?)
        }
    };

    Ok(ExecOutput {
        success: status.success(),
        stdout,
        stderr,
    })
}

pub fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::new(io::ErrorKind::Other, err)
}

/// Determines if using windows by checking the OS environment variable
pub async fn is_windows(session: &Session) -> io::Result<bool> {
    let output = execute_output(session, "cmd.exe /C echo %OS%", SSH_EXEC_TIMEOUT).await?;

    fn contains_subslice(slice: &[u8], subslice: &[u8]) -> bool {
        for i in 0..slice.len() {
            if i + subslice.len() > slice.len() {
                break;
            }

            if slice[i..].starts_with(subslice) {
                return true;
            }
        }

        false
    }

    Ok(contains_subslice(&output.stdout, b"Windows_NT")
        || contains_subslice(&output.stderr, b"Windows_NT"))
}

/// Query remote system for name of current user
pub async fn query_username(session: &Session, is_windows: bool) -> io::Result<String> {
    let output = if is_windows {
        execute_output(session, "cmd.exe /C echo %username%", SSH_EXEC_TIMEOUT).await?
    } else {
        execute_output(session, "/bin/sh -c whoami", SSH_EXEC_TIMEOUT).await?
    };

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Query remote system for the default shell of current user
pub async fn query_shell(session: &Session, is_windows: bool) -> io::Result<String> {
    let output = if is_windows {
        execute_output(session, "cmd.exe /C echo %ComSpec%", SSH_EXEC_TIMEOUT).await?
    } else {
        execute_output(session, "/bin/sh -c 'echo $SHELL'", SSH_EXEC_TIMEOUT).await?
    };

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Attempts to convert UTF8 str into a path compliant with Windows
pub fn convert_to_windows_path(s: &str) -> Option<PathBuf> {
    let path = WindowsPath::new(s);
    let mut components = path.components();

    // If we start with a root directory, we may have the weird path
    match components.next() {
        // Something weird like /C:/... or /C/... that we need to convert to C:\...
        Some(WindowsComponent::RootDir) => {
            let path = WindowsPath::new(components.as_bytes());

            // If we have a prefix, then that means we had something like /C:/...
            if let Some(WindowsComponent::Prefix(_)) = path.components().next() {
                std::str::from_utf8(path.as_bytes()).ok().map(PathBuf::from)
            } else if let Some(WindowsComponent::Normal(filename)) = components.next() {
                // If we have a drive letter, convert it into a path
                // /C/... -> C:\...
                if filename.len() == 1 && (filename[0] as char).is_alphabetic() {
                    let mut path_buf = WindowsPathBuf::from(format!("{}:", filename[0]));
                    for component in components {
                        path_buf.push(component);
                    }
                    std::str::from_utf8(path.as_bytes()).ok().map(PathBuf::from)
                } else {
                    None
                }
            } else {
                None
            }
        }

        // Already is a Windows path, so just wrap str in std PathBuf
        Some(WindowsComponent::Prefix(_)) => Some(PathBuf::from(s)),

        // Not a reliable Windows path, so return None
        _ => None,
    }
}

/// Performs canonicalization of the given path using SFTP
pub async fn canonicalize(sftp: &Sftp, path: impl AsRef<Path>) -> io::Result<PathBuf> {
    sftp.canonicalize(path.as_ref().to_path_buf())
        .compat()
        .await
        .map(|p| p.into_std_path_buf())
        .map_err(to_other_error)
}
