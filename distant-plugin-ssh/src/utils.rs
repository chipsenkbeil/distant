use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{fmt, io};

use async_compat::CompatExt;
use typed_path::windows::WindowsComponent;
use typed_path::{Components, WindowsPath, WindowsPathBuf};
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

pub async fn powershell_output(
    session: &Session,
    cmd: &str,
    timeout: impl Into<Option<Duration>>,
) -> io::Result<ExecOutput> {
    let cmd = format!("powershell.exe -NonInteractive -Command \"& {{{cmd}}}\"");
    execute_output(session, &cmd, timeout).await
}

pub async fn execute_output(
    session: &Session,
    cmd: &str,
    timeout: impl Into<Option<Duration>>,
) -> io::Result<ExecOutput> {
    let timeout = timeout.into();
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

    // NOTE: There is a bug where if the ssh backend is libssh, the non-blocking readers
    //       will never report Ok(0) and are always Err(WouldBlock). So, we want to track
    //       when a process exits and then cancel the readers if we receive Err(Wouldblock)
    let (tx, rx) = tokio::sync::watch::channel(false);

    macro_rules! spawn_reader {
        ($reader:ident) => {{
            let rx = rx.clone();
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
                            // NOTE: This only exists because of the above bug with libssh!
                            if *rx.borrow() {
                                break Ok(bytes);
                            }

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

    // Notify our handles that we are done
    let _ = tx.send(true);

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
    let output = powershell_output(
        session,
        "[Environment]::GetEnvironmentVariable('OS')",
        SSH_EXEC_TIMEOUT,
    )
    .await?;

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
    if is_windows {
        // Will get DOMAIN\USERNAME as output -- needed because USERNAME isn't set on
        // Github's Windows CI (it sets USER instead)
        let output = powershell_output(
            session,
            "[System.Security.Principal.WindowsIdentity]::GetCurrent().Name",
            SSH_EXEC_TIMEOUT,
        )
        .await?;

        let output = String::from_utf8_lossy(&output.stdout);
        let output = match output.split_once('\\') {
            Some((_, username)) => username,
            None => output.as_ref(),
        };

        Ok(output.trim().to_string())
    } else {
        let output = execute_output(session, "/bin/sh -c whoami", SSH_EXEC_TIMEOUT).await?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Query remote system for the default shell of current user
pub async fn query_shell(session: &Session, is_windows: bool) -> io::Result<String> {
    let output = if is_windows {
        powershell_output(
            session,
            "[Environment]::GetEnvironmentVariable('ComSpec')",
            SSH_EXEC_TIMEOUT,
        )
        .await?
    } else {
        execute_output(session, "/bin/sh -c 'echo $SHELL'", SSH_EXEC_TIMEOUT).await?
    };

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Attempts to convert UTF8 str into a path compliant with Windows
pub fn convert_to_windows_path_string(s: &str) -> Option<String> {
    let path = WindowsPath::new(s);
    let mut components = path.components();

    // If we start with a root directory, we may have the weird path
    match components.next() {
        // Something weird like /C:/... or /C/... that we need to convert to C:\...
        Some(WindowsComponent::RootDir) => {
            let path = WindowsPath::new(components.as_bytes());

            // If we have a prefix, then that means we had something like /C:/...
            if let Some(WindowsComponent::Prefix(_)) = path.components().next() {
                std::str::from_utf8(path.as_bytes())
                    .ok()
                    .map(ToString::to_string)
            } else if let Some(WindowsComponent::Normal(filename)) = components.next() {
                // If we have a drive letter, convert it into a path, e.g. /C/... -> C:\...
                if filename.len() == 1 && (filename[0] as char).is_alphabetic() {
                    let mut path_buf = WindowsPathBuf::from(format!("{}:", filename[0]));
                    for component in components {
                        path_buf.push(component);
                    }
                    std::str::from_utf8(path.as_bytes())
                        .ok()
                        .map(ToString::to_string)
                } else {
                    None
                }
            } else {
                None
            }
        }

        // Already is a Windows path, so just return string
        Some(WindowsComponent::Prefix(_)) => Some(s.to_string()),

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
