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

/// Strips terminal query response sequences from a byte buffer.
///
/// When the terminal is in raw mode, programs like nvim send escape sequences
/// to query the terminal (e.g. cursor position via DSR, device attributes via
/// DA1/DA2). The terminal emulator responds by writing the answer back on stdin.
/// These responses must NOT be forwarded to the remote process — they would
/// confuse the remote program's input parser.
///
/// The old termwiz-based code parsed stdin into structured events and silently
/// dropped non-key, non-resize events (which included these responses). This
/// function restores that behavior for raw byte forwarding.
///
/// Response patterns filtered:
/// - `\x1b[...R`     — DSR (Cursor Position Report)
/// - `\x1b[?...c`    — DA1 (Primary Device Attributes)
/// - `\x1b[>...c`    — DA2 (Secondary Device Attributes)
/// - `\x1b[?...;...$y` — DECRPM (Mode Report)
///
/// Returns the number of bytes written to `out` after filtering.
fn filter_terminal_responses(input: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < input.len() {
        if input[i] == 0x1b && i + 1 < input.len() && input[i + 1] == b'[' {
            // Potential CSI sequence — scan for the terminator
            if let Some(seq_len) = scan_csi_response(&input[i..]) {
                trace!(
                    "Filtered terminal response: {:02x?}",
                    &input[i..i + seq_len]
                );
                i += seq_len;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
}

/// Scans a CSI sequence starting at `buf[0] == ESC, buf[1] == '['` and returns
/// its length if it matches a terminal query response pattern. Returns `None` if
/// it's not a recognized response (i.e. should be forwarded as user input).
fn scan_csi_response(buf: &[u8]) -> Option<usize> {
    // Minimum: ESC [ <something> <terminator> = 3 bytes
    if buf.len() < 3 || buf[0] != 0x1b || buf[1] != b'[' {
        return None;
    }

    let mut j = 2;

    // Check for '?' or '>' prefix (DA1: `ESC[?...c`, DA2: `ESC[>...c`)
    let has_question = buf.get(j) == Some(&b'?');
    let has_gt = buf.get(j) == Some(&b'>');
    if has_question || has_gt {
        j += 1;
    }

    // Scan parameter bytes (digits and semicolons)
    while j < buf.len() && (buf[j].is_ascii_digit() || buf[j] == b';') {
        j += 1;
    }

    if j >= buf.len() {
        // Incomplete sequence — don't consume, let it accumulate
        return None;
    }

    let terminator = buf[j];

    // DSR response: ESC [ <digits> ; <digits> R
    if terminator == b'R' && !has_question && !has_gt {
        return Some(j + 1);
    }

    // DA1 response: ESC [ ? <params> c
    // DA2 response: ESC [ > <params> c
    if terminator == b'c' && (has_question || has_gt) {
        return Some(j + 1);
    }

    // DECRPM response: ESC [ ? <params> $ y
    if terminator == b'$' && has_question && j + 1 < buf.len() && buf[j + 1] == b'y' {
        return Some(j + 2);
    }

    None
}

/// RAII guard that restores the original `fcntl` flags on stdin when dropped.
#[cfg(unix)]
struct NonBlockGuard {
    fd: std::os::fd::RawFd,
    original_flags: libc::c_int,
}

#[cfg(unix)]
impl Drop for NonBlockGuard {
    fn drop(&mut self) {
        unsafe {
            libc::fcntl(self.fd, libc::F_SETFL, self.original_flags);
        }
    }
}

/// Forwards raw stdin bytes to the remote process stdin.
///
/// On Unix, uses `AsyncFd` with non-blocking I/O so the task can be cleanly
/// cancelled when the remote process exits. Terminal query responses (DSR, DA1,
/// DA2, DECRPM) are filtered out before forwarding — see [`filter_terminal_responses`].
///
/// On Windows, uses `tokio::io::stdin()` which blocks a thread internally —
/// the caller must handle process exit separately.
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
    let _guard = NonBlockGuard {
        fd: raw_fd,
        original_flags,
    };

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
    let mut filtered = Vec::with_capacity(4096);

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
                trace!("stdin: {} bytes: {:02x?}", n, &buf[..n.min(64)]);
                filtered.clear();
                filter_terminal_responses(&buf[..n], &mut filtered);
                if !filtered.is_empty() && writer.write(filtered.as_slice()).await.is_err() {
                    break;
                }
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Ok(Err(_)) => break,
            Err(_would_block) => continue,
        }
    }
}

/// Forwards raw stdin bytes to the remote process stdin (Windows version).
///
/// Terminal query responses are filtered out before forwarding.
#[cfg(windows)]
async fn forward_stdin(mut writer: distant_core::RemoteStdin) {
    let mut buf = [0u8; 4096];
    let mut filtered = Vec::with_capacity(4096);
    let mut reader = tokio::io::stdin();
    loop {
        match tokio::io::AsyncReadExt::read(&mut reader, &mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                trace!("stdin: {} bytes: {:02x?}", n, &buf[..n.min(64)]);
                filtered.clear();
                filter_terminal_responses(&buf[..n], &mut filtered);
                if !filtered.is_empty() && writer.write(filtered.as_slice()).await.is_err() {
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

    // ─── filter_terminal_responses tests ───

    #[test]
    fn filter_passes_through_normal_text() {
        let input = b"hello world";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert_eq!(out, b"hello world");
    }

    #[test]
    fn filter_passes_through_normal_key_escapes() {
        // Arrow keys: ESC [ A/B/C/D — not terminal responses
        let input = b"\x1b[A\x1b[B";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    #[test]
    fn filter_strips_dsr_cursor_position_report() {
        // DSR response: ESC [ 24 ; 80 R
        let input = b"\x1b[24;80R";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert!(out.is_empty(), "DSR response should be filtered: {:?}", out);
    }

    #[test]
    fn filter_strips_da1_response() {
        // DA1 response: ESC [ ? 6 4 ; 1 ; 2 c
        let input = b"\x1b[?64;1;2c";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert!(out.is_empty(), "DA1 response should be filtered: {:?}", out);
    }

    #[test]
    fn filter_strips_da2_response() {
        // DA2 response: ESC [ > 1 ; 1 0 ; 0 c
        let input = b"\x1b[>1;10;0c";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert!(out.is_empty(), "DA2 response should be filtered: {:?}", out);
    }

    #[test]
    fn filter_strips_decrpm_response() {
        // DECRPM response: ESC [ ? 25 ; 1 $ y
        let input = b"\x1b[?25;1$y";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert!(
            out.is_empty(),
            "DECRPM response should be filtered: {:?}",
            out
        );
    }

    #[test]
    fn filter_preserves_text_around_responses() {
        // Normal text, then DSR, then more text
        let mut input = Vec::new();
        input.extend_from_slice(b"hello");
        input.extend_from_slice(b"\x1b[24;80R");
        input.extend_from_slice(b"world");

        let mut out = Vec::new();
        filter_terminal_responses(&input, &mut out);
        assert_eq!(out, b"helloworld");
    }

    #[test]
    fn filter_handles_multiple_responses_in_sequence() {
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b[24;80R"); // DSR
        input.extend_from_slice(b"\x1b[?64;1c"); // DA1
        input.extend_from_slice(b"x"); // regular byte

        let mut out = Vec::new();
        filter_terminal_responses(&input, &mut out);
        assert_eq!(out, b"x");
    }

    #[test]
    fn filter_passes_through_csi_sequences_that_are_not_responses() {
        // SGR (color): ESC [ 3 1 m — not a response
        let input = b"\x1b[31m";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    // ─── scan_csi_response tests ───

    #[test]
    fn scan_csi_returns_none_for_non_csi() {
        assert!(scan_csi_response(b"hello").is_none());
        assert!(scan_csi_response(b"\x1bO").is_none());
    }

    #[test]
    fn scan_csi_returns_none_for_incomplete_sequence() {
        assert!(scan_csi_response(b"\x1b[").is_none());
        assert!(scan_csi_response(b"\x1b[24;").is_none());
    }

    #[test]
    fn scan_csi_returns_length_for_dsr() {
        assert_eq!(scan_csi_response(b"\x1b[24;80R"), Some(8));
        assert_eq!(scan_csi_response(b"\x1b[1;1R"), Some(6));
    }

    #[test]
    fn scan_csi_returns_length_for_da1() {
        assert_eq!(scan_csi_response(b"\x1b[?64;1;2c"), Some(10));
    }

    #[test]
    fn scan_csi_returns_length_for_da2() {
        assert_eq!(scan_csi_response(b"\x1b[>1;10;0c"), Some(10));
    }

    #[test]
    fn scan_csi_returns_length_for_decrpm() {
        assert_eq!(scan_csi_response(b"\x1b[?25;1$y"), Some(9));
    }

    #[test]
    fn scan_csi_returns_none_for_arrow_keys() {
        // Arrow up: ESC [ A
        assert!(scan_csi_response(b"\x1b[A").is_none());
    }
}
