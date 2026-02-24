use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use distant_core::protocol::{Environment, PtySize};
use distant_core::{Channel, ChannelExt, RemoteCommand};
use log::*;
use terminal_size::{terminal_size, Height, Width};
use termwiz::caps::Capabilities;
use termwiz::input::{InputEvent, KeyCodeEncodeModes, KeyboardEncoding};
use termwiz::terminal::{new_terminal, Terminal};

use super::super::common::RemoteProcessLink;
use super::{CliError, CliResult};

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
            .pty(
                terminal_size()
                    .map(|(Width(cols), Height(rows))| PtySize::from_rows_and_cols(rows, cols)),
            )
            .current_dir(current_dir)
            .spawn(self.0, &cmd)
            .await
            .with_context(|| format!("Failed to spawn {cmd}"))?;

        // Create a new terminal in raw mode
        let mut terminal = new_terminal(
            Capabilities::new_from_env().context("Failed to load terminal capabilities")?,
        )
        .context("Failed to create terminal")?;
        terminal.set_raw_mode().context("Failed to set raw mode")?;

        let mut stdin = proc.stdin.take().unwrap();
        let resizer = proc.clone_resizer();
        tokio::spawn(async move {
            while let Ok(input) = terminal.poll_input(Some(Duration::new(0, 0))) {
                match input {
                    Some(InputEvent::Key(ev)) => {
                        if let Ok(input) = ev.key.encode(
                            ev.modifiers,
                            KeyCodeEncodeModes {
                                encoding: KeyboardEncoding::Xterm,
                                application_cursor_keys: false,
                                newline_mode: false,
                                modify_other_keys: None,
                            },
                            /* is_down */ true,
                        ) {
                            if let Err(x) = stdin.write_str(input).await {
                                error!("Failed to write to stdin of remote process: {}", x);
                                break;
                            }
                        }
                    }
                    Some(InputEvent::Resized { cols, rows }) => {
                        if let Err(x) = resizer
                            .resize(PtySize::from_rows_and_cols(rows as u16, cols as u16))
                            .await
                        {
                            error!("Failed to resize remote process: {}", x);
                            break;
                        }
                    }
                    Some(_) => continue,
                    None => tokio::time::sleep(Duration::from_millis(1)).await,
                }
            }
        });

        // Now, map the remote shell's stdout/stderr to our own process,
        // while stdin is handled by the task above
        let link = RemoteProcessLink::from_remote_pipes(
            None,
            proc.stdout.take().unwrap(),
            proc.stderr.take().unwrap(),
            max_chunk_size,
        );

        // Continually loop to check for terminal resize changes while the process is still running
        let status = proc.wait().await.context("Failed to wait for process")?;

        // Shut down our link
        link.shutdown().await;

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
