use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use distant_core::protocol::{Environment, PtySize, RemotePath};
use distant_core::{Channel, ChannelExt, RemoteCommand};
use log::*;
use terminal_size::{Height, Width, terminal_size};
use termwiz::caps::Capabilities;
use termwiz::input::{InputEvent, KeyCodeEncodeModes, KeyboardEncoding};
use termwiz::terminal::{Terminal, new_terminal};

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

/// Strips ConPTY-specific escape sequences from stdout before they reach the local terminal.
///
/// When connecting to a Windows host, ConPTY emits terminal mode sequences that are
/// meaningful only to Windows console input but cause problems when forwarded to a
/// macOS/Linux terminal emulator through the shell proxy:
///
/// - `ESC[?9001h/l` (Win32 input mode) — not understood by non-Windows terminals;
///   may confuse ConPTY's input parser if it believes the mode was acknowledged.
/// - `ESC[?1004h/l` (focus tracking) — causes the local terminal to generate
///   `ESC[I`/`ESC[O` focus events on stdin, which are noise for the remote process.
/// - `ESC[8;<rows>;<cols>t` (XTWINOPS resize) — causes the local terminal to resize
///   its window, triggering SIGWINCH and a resize feedback loop.
///
/// All other sequences (cursor movement, SGR colors, standard DEC modes like
/// `ESC[?25h`) are passed through unchanged.
fn filter_conpty_stdout(input: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < input.len() {
        if input[i] == 0x1b
            && i + 1 < input.len()
            && input[i + 1] == b'['
            && let Some(seq_len) = scan_conpty_sequence(&input[i..])
        {
            trace!(
                "Filtered ConPTY stdout sequence: {:02x?}",
                &input[i..i + seq_len]
            );
            i += seq_len;
            continue;
        }
        out.push(input[i]);
        i += 1;
    }
}

/// Scans for a ConPTY-specific sequence starting at `buf[0] == ESC, buf[1] == '['`.
///
/// Returns the total byte length of the sequence if it matches a known ConPTY pattern,
/// or `None` if it should be passed through.
fn scan_conpty_sequence(buf: &[u8]) -> Option<usize> {
    if buf.len() < 3 || buf[0] != 0x1b || buf[1] != b'[' {
        return None;
    }

    // ESC[?<number>h or ESC[?<number>l — private mode set/reset
    if buf[2] == b'?' {
        let mut j = 3;
        while j < buf.len() && buf[j].is_ascii_digit() {
            j += 1;
        }
        if j >= buf.len() || j == 3 {
            return None; // Incomplete or no digits
        }
        if buf[j] == b'h' || buf[j] == b'l' {
            let mode_str = std::str::from_utf8(&buf[3..j]).ok()?;
            let mode: u32 = mode_str.parse().ok()?;
            if mode == 9001 || mode == 1004 {
                return Some(j + 1);
            }
        }
        return None;
    }

    // ESC[8;<digits>;<digits>t — XTWINOPS resize
    if buf[2] == b'8' && buf.len() > 3 && buf[3] == b';' {
        let mut j = 4;
        // First parameter: rows (digits)
        let start = j;
        while j < buf.len() && buf[j].is_ascii_digit() {
            j += 1;
        }
        if j == start || j >= buf.len() || buf[j] != b';' {
            return None;
        }
        j += 1; // skip ';'
        // Second parameter: cols (digits)
        let start = j;
        while j < buf.len() && buf[j].is_ascii_digit() {
            j += 1;
        }
        if j == start || j >= buf.len() || buf[j] != b't' {
            return None;
        }
        return Some(j + 1); // include 't'
    }

    None
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
            .current_dir(current_dir.map(RemotePath::from))
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
                        ) && let Err(x) = stdin.write_str(input).await
                        {
                            error!("Failed to write to stdin of remote process: {}", x);
                            break;
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
        // while stdin is handled by the task above.
        // Filter ConPTY-specific sequences from stdout to prevent focus tracking
        // leaks and resize feedback loops when connecting to Windows hosts.
        let link = RemoteProcessLink::from_remote_pipes_filtered(
            None,
            proc.stdout.take().unwrap(),
            proc.stderr.take().unwrap(),
            max_chunk_size,
            filter_conpty_stdout,
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

    // ─── filter_conpty_stdout tests ───

    #[test]
    fn filter_strips_focus_tracking_enable() {
        let input = b"\x1b[?1004h";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(out.is_empty(), "should strip ?1004h: {out:?}");
    }

    #[test]
    fn filter_strips_focus_tracking_disable() {
        let input = b"\x1b[?1004l";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(out.is_empty(), "should strip ?1004l: {out:?}");
    }

    #[test]
    fn filter_strips_win32_input_mode_enable() {
        let input = b"\x1b[?9001h";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(out.is_empty(), "should strip ?9001h: {out:?}");
    }

    #[test]
    fn filter_strips_win32_input_mode_disable() {
        let input = b"\x1b[?9001l";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(out.is_empty(), "should strip ?9001l: {out:?}");
    }

    #[test]
    fn filter_strips_xtwinops_resize() {
        let input = b"\x1b[8;24;80t";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(out.is_empty(), "should strip XTWINOPS: {out:?}");
    }

    #[test]
    fn filter_passes_through_normal_text() {
        let input = b"hello world";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    #[test]
    fn filter_passes_through_standard_csi_sequences() {
        // SGR color: ESC[1;31m
        let input = b"\x1b[1;31m";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert_eq!(out, input.to_vec(), "SGR should pass through");

        // Show cursor: ESC[?25h (mode 25 is not 1004 or 9001)
        let input = b"\x1b[?25h";
        out.clear();
        filter_conpty_stdout(input, &mut out);
        assert_eq!(out, input.to_vec(), "?25h should pass through");
    }

    #[test]
    fn filter_handles_mixed_content() {
        let mut input = Vec::new();
        input.extend_from_slice(b"hello");
        input.extend_from_slice(b"\x1b[?1004h");
        input.extend_from_slice(b"world");

        let mut out = Vec::new();
        filter_conpty_stdout(&input, &mut out);
        assert_eq!(out, b"helloworld");
    }

    #[test]
    fn filter_handles_empty_input() {
        let mut out = Vec::new();
        filter_conpty_stdout(b"", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn filter_handles_multiple_conpty_sequences() {
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b[?9001h");
        input.extend_from_slice(b"\x1b[?1004h");
        input.extend_from_slice(b"\x1b[8;50;120t");
        input.extend_from_slice(b"ok");

        let mut out = Vec::new();
        filter_conpty_stdout(&input, &mut out);
        assert_eq!(out, b"ok");
    }

    #[test]
    fn filter_passes_incomplete_esc_at_end() {
        // A lone ESC at the end — should pass through (not a complete sequence)
        let input = b"text\x1b";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    // ─── scan_conpty_sequence tests ───

    #[test]
    fn scan_returns_none_for_non_csi() {
        assert!(scan_conpty_sequence(b"hello").is_none());
        assert!(scan_conpty_sequence(b"\x1bO").is_none());
    }

    #[test]
    fn scan_returns_none_for_standard_private_modes() {
        // ?25h (show cursor) — not a ConPTY-specific mode
        assert!(scan_conpty_sequence(b"\x1b[?25h").is_none());
        // ?1049h (alternate screen) — not ConPTY-specific
        assert!(scan_conpty_sequence(b"\x1b[?1049h").is_none());
    }

    #[test]
    fn scan_returns_length_for_1004h() {
        assert_eq!(scan_conpty_sequence(b"\x1b[?1004h"), Some(8));
    }

    #[test]
    fn scan_returns_length_for_9001l() {
        assert_eq!(scan_conpty_sequence(b"\x1b[?9001l"), Some(8));
    }

    #[test]
    fn scan_returns_length_for_xtwinops() {
        assert_eq!(scan_conpty_sequence(b"\x1b[8;24;80t"), Some(10));
        assert_eq!(scan_conpty_sequence(b"\x1b[8;50;120t"), Some(11));
    }

    #[test]
    fn scan_returns_none_for_incomplete_sequence() {
        assert!(scan_conpty_sequence(b"\x1b[").is_none());
        assert!(scan_conpty_sequence(b"\x1b[?").is_none());
        assert!(scan_conpty_sequence(b"\x1b[?1004").is_none());
    }
}
