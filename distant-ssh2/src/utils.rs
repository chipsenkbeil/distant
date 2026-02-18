use std::io;
use std::time::Duration;

use russh::client::Handle;
use typed_path::{Components, WindowsComponent, WindowsPath, WindowsPathBuf};

use crate::ClientHandler;

const SSH_EXEC_TIMEOUT: Option<Duration> = Some(Duration::from_secs(30));

#[allow(dead_code)]
const READER_PAUSE_MILLIS: u64 = 100;

#[derive(Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl std::fmt::Debug for ExecOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    handle: &Handle<ClientHandler>,
    cmd: &str,
    timeout: impl Into<Option<Duration>>,
) -> io::Result<ExecOutput> {
    let cmd = format!("powershell.exe -NonInteractive -Command \"& {{{cmd}}}\"");
    execute_output(handle, &cmd, timeout).await
}

pub async fn execute_output(
    handle: &Handle<ClientHandler>,
    cmd: &str,
    timeout: impl Into<Option<Duration>>,
) -> io::Result<ExecOutput> {
    use russh::ChannelMsg;

    let timeout_duration = timeout.into();

    // Open a channel
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(to_other_error)?;

    // Execute command
    channel.exec(true, cmd).await.map_err(to_other_error)?;

    let read_future = async {
        // Read output via channel messages
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status: Option<u32> = None;
        let mut got_eof = false;

        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { ref data } => {
                    stdout.extend_from_slice(data);
                }
                ChannelMsg::ExtendedData { ref data, ext } => {
                    if ext == 1 {
                        stderr.extend_from_slice(data);
                    }
                }
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => {
                    exit_status = Some(status);
                    if got_eof {
                        break;
                    }
                }
                ChannelMsg::Eof => {
                    got_eof = true;
                    if exit_status.is_some() {
                        break;
                    }
                }
                _ => {}
            }
        }

        Ok(ExecOutput {
            success: exit_status.map(|s| s == 0).unwrap_or(false),
            stdout,
            stderr,
        })
    };

    if let Some(duration) = timeout_duration {
        tokio::time::timeout(duration, read_future)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "SSH command timed out"))?
    } else {
        read_future.await
    }
}

/// Query remote system for name of current user
pub async fn query_username(
    handle: &Handle<ClientHandler>,
    is_windows: bool,
) -> io::Result<String> {
    if is_windows {
        // Will get DOMAIN\USERNAME as output -- needed because USERNAME isn't set on
        // Github's Windows CI (it sets USER instead)
        let output = powershell_output(
            handle,
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
        let output = execute_output(handle, "/bin/sh -c whoami", SSH_EXEC_TIMEOUT).await?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Query remote system for the default shell of current user
pub async fn query_shell(handle: &Handle<ClientHandler>, is_windows: bool) -> io::Result<String> {
    let output = if is_windows {
        powershell_output(
            handle,
            "[Environment]::GetEnvironmentVariable('ComSpec')",
            SSH_EXEC_TIMEOUT,
        )
        .await?
    } else {
        execute_output(handle, "/bin/sh -c 'echo $SHELL'", SSH_EXEC_TIMEOUT).await?
    };

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn to_other_error<E>(err: E) -> io::Error
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    io::Error::other(err)
}

/// Determines if using windows by checking the OS environment variable
pub async fn is_windows(handle: &Handle<ClientHandler>) -> io::Result<bool> {
    let output = powershell_output(
        handle,
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
