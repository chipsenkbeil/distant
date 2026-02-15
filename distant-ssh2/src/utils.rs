use std::io;
use std::time::Duration;

use russh::client::Handle;
use typed_path::{Components, WindowsComponent, WindowsPath, WindowsPathBuf};

use crate::ClientHandler;

const SSH_EXEC_TIMEOUT: Option<Duration> = Some(Duration::from_secs(1));

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
    _timeout: impl Into<Option<Duration>>,
) -> io::Result<ExecOutput> {
    use russh::ChannelMsg;

    // Open a channel
    let mut channel = handle
        .channel_open_session()
        .await
        .map_err(to_other_error)?;

    // Execute command
    channel.exec(true, cmd).await.map_err(to_other_error)?;

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
                    // stderr
                    stderr.extend_from_slice(data);
                }
            }
            ChannelMsg::ExitStatus {
                exit_status: status,
            } => {
                exit_status = Some(status);
                // If we already got EOF, we can exit now
                if got_eof {
                    break;
                }
            }
            ChannelMsg::Eof => {
                got_eof = true;
                // If we already got exit status, we can exit now
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
