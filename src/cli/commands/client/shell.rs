use std::path::PathBuf;

use anyhow::Context;
use distant_core::protocol::{Environment, PtySize, RemotePath};
use distant_core::{Channel, ChannelExt, RemoteCommand};
use log::*;

use super::super::common::RemoteProcessLink;
use super::{CliError, CliResult};
use crate::cli::common::terminal::{RawMode, terminal_size, wait_for_resize};

/// Inserts `TERM=xterm-256color` into the environment if no `TERM` key is present.
fn ensure_term_env(env: &mut Environment) {
    if !env.contains_key("TERM") {
        env.insert("TERM".to_string(), "xterm-256color".to_string());
    }
}

/// Selects a default shell: returns `shell` if non-empty, otherwise falls back based on OS family.
fn select_default_shell(shell: &str, family: &str) -> String {
    if !shell.is_empty() {
        shell.to_string()
    } else if family.eq_ignore_ascii_case("windows") {
        "cmd.exe".to_string()
    } else {
        "/bin/sh".to_string()
    }
}

/// Forwards raw stdin bytes to the remote process stdin.
///
/// On Unix, uses `AsyncFd` with non-blocking I/O so the task can be cleanly
/// cancelled when the remote process exits. On Windows, uses `tokio::io::stdin()`
/// which blocks a thread internally — the caller must handle process exit separately.
#[cfg(unix)]
async fn forward_stdin(mut writer: distant_core::RemoteStdin) {
    use std::os::fd::AsRawFd;
    use tokio::io::unix::AsyncFd;

    let raw_fd = std::io::stdin().as_raw_fd();

    // Set stdin to non-blocking so we can use AsyncFd
    let original_flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFL) };
    unsafe {
        libc::fcntl(raw_fd, libc::F_SETFL, original_flags | libc::O_NONBLOCK);
    }

    // Safety: StdinFd is a trivial wrapper that implements AsRawFd
    struct StdinFd(std::os::fd::RawFd);
    impl AsRawFd for StdinFd {
        fn as_raw_fd(&self) -> std::os::fd::RawFd {
            self.0
        }
    }

    let Ok(async_fd) = AsyncFd::new(StdinFd(raw_fd)) else {
        return;
    };
    let mut buf = [0u8; 4096];

    loop {
        let mut guard = match async_fd.readable().await {
            Ok(g) => g,
            Err(_) => break,
        };
        match guard.try_io(|inner| {
            let fd = inner.get_ref().as_raw_fd();
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(n as usize)
            }
        }) {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                if writer.write(&buf[..n]).await.is_err() {
                    break;
                }
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Ok(Err(_)) => break,
            Err(_would_block) => continue,
        }
    }

    // Restore blocking mode
    unsafe {
        libc::fcntl(raw_fd, libc::F_SETFL, original_flags);
    }
}

/// Forwards raw stdin bytes to the remote process stdin (Windows version).
#[cfg(windows)]
async fn forward_stdin(mut writer: distant_core::RemoteStdin) {
    let mut buf = [0u8; 4096];
    let mut reader = tokio::io::stdin();
    loop {
        match tokio::io::AsyncReadExt::read(&mut reader, &mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if writer.write(&buf[..n]).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[derive(Clone)]
pub struct Shell(Channel);

impl Shell {
    pub fn new(channel: Channel) -> Self {
        Self(channel)
    }

    pub async fn spawn(
        mut self,
        cmd: impl Into<Option<String>>,
        mut environment: Environment,
        current_dir: Option<PathBuf>,
        max_chunk_size: usize,
    ) -> CliResult {
        ensure_term_env(&mut environment);

        // Use provided shell, use default shell, or determine remote operating system to pick a shell
        let cmd = match cmd.into() {
            Some(cmd) => cmd,
            None => {
                let system_info = self
                    .0
                    .system_info()
                    .await
                    .context("Failed to detect remote operating system")?;

                select_default_shell(&system_info.shell, &system_info.family)
            }
        };

        let mut proc = RemoteCommand::new()
            .environment(environment)
            .pty(terminal_size().map(|(cols, rows)| PtySize::from_rows_and_cols(rows, cols)))
            .current_dir(current_dir.map(RemotePath::from))
            .spawn(self.0, &cmd)
            .await
            .with_context(|| format!("Failed to spawn {cmd}"))?;

        // Enter raw mode — restored automatically when _raw_mode guard is dropped
        let _raw_mode = RawMode::enter().context("Failed to set raw mode")?;

        // Forward raw stdin bytes to the remote process
        let stdin = proc.stdin.take().unwrap();
        let stdin_task = tokio::spawn(forward_stdin(stdin));

        // Detect terminal resize events and forward to the remote PTY
        let resizer = proc.clone_resizer();
        let resize_task = tokio::spawn(async move {
            while let Some((cols, rows)) = wait_for_resize().await {
                if let Err(x) = resizer
                    .resize(PtySize::from_rows_and_cols(rows, cols))
                    .await
                {
                    error!("Failed to resize remote process: {}", x);
                    break;
                }
            }
        });

        // Map the remote shell's stdout/stderr to our own process
        let link = RemoteProcessLink::from_remote_pipes(
            None,
            proc.stdout.take().unwrap(),
            proc.stderr.take().unwrap(),
            max_chunk_size,
        );

        let status = proc.wait().await.context("Failed to wait for process")?;

        // Abort background tasks so the process can exit cleanly
        stdin_task.abort();
        resize_task.abort();

        // Shut down our link
        link.shutdown().await;

        // _raw_mode dropped here, restoring terminal state

        if !status.success {
            if let Some(code) = status.code {
                return Err(CliError::Exit(code as u8));
            } else {
                return Err(CliError::FAILURE);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── ensure_term_env tests ───

    #[test]
    fn ensure_term_env_inserts_when_missing() {
        let mut env = Environment::new();
        ensure_term_env(&mut env);
        assert_eq!(env.get("TERM").unwrap(), "xterm-256color");
    }

    #[test]
    fn ensure_term_env_preserves_existing() {
        let mut env = Environment::new();
        env.insert("TERM".to_string(), "screen".to_string());
        ensure_term_env(&mut env);
        assert_eq!(env.get("TERM").unwrap(), "screen");
    }

    #[test]
    fn ensure_term_env_preserves_other_keys() {
        let mut env = Environment::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        ensure_term_env(&mut env);
        assert_eq!(env.get("PATH").unwrap(), "/usr/bin");
        assert_eq!(env.get("TERM").unwrap(), "xterm-256color");
    }

    // ─── select_default_shell tests ───

    #[test]
    fn select_default_shell_uses_provided_shell() {
        assert_eq!(select_default_shell("/bin/zsh", "linux"), "/bin/zsh");
    }

    #[test]
    fn select_default_shell_windows_fallback() {
        assert_eq!(select_default_shell("", "windows"), "cmd.exe");
    }

    #[test]
    fn select_default_shell_unix_fallback() {
        assert_eq!(select_default_shell("", "linux"), "/bin/sh");
    }

    #[test]
    fn select_default_shell_case_insensitive_windows() {
        assert_eq!(select_default_shell("", "Windows"), "cmd.exe");
        assert_eq!(select_default_shell("", "WINDOWS"), "cmd.exe");
    }

    #[test]
    fn select_default_shell_unknown_family_defaults_to_sh() {
        assert_eq!(select_default_shell("", "macos"), "/bin/sh");
        assert_eq!(select_default_shell("", "freebsd"), "/bin/sh");
    }

    #[test]
    fn select_default_shell_ignores_family_when_shell_provided() {
        assert_eq!(
            select_default_shell("powershell.exe", "windows"),
            "powershell.exe"
        );
    }
}
