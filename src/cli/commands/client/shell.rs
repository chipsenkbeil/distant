use std::path::PathBuf;

use anyhow::Context;
use distant_core::protocol::{Environment, PtySize, RemotePath};
use distant_core::{Channel, ChannelExt, RemoteCommand};
use log::*;

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

/// Filters unsolicited terminal events from stdin before forwarding to the remote process.
///
/// When the local terminal has focus tracking enabled (via `ESC[?1004h` sent by
/// the remote program on stdout), the terminal emulator generates focus in/out
/// events on stdin. These are local-only notifications and must not be forwarded
/// to the remote process.
///
/// Terminal query responses (DSR, DA1, DA2, DECRPM) are intentionally passed
/// through — they are legitimate answers to questions the remote program asked,
/// and filtering them causes TUI programs like neovim to hang waiting for
/// responses that never arrive.
///
/// Filtered patterns:
/// - `\x1b[I` — Focus In event
/// - `\x1b[O` — Focus Out event
fn filter_terminal_responses(input: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < input.len() {
        // Focus events: ESC[I (focus in) and ESC[O (focus out)
        if input[i] == 0x1b
            && i + 2 < input.len()
            && input[i + 1] == b'['
            && (input[i + 2] == b'I' || input[i + 2] == b'O')
        {
            trace!("Filtered focus event: {:02x?}", &input[i..i + 3]);
            i += 3;
            continue;
        }
        out.push(input[i]);
        i += 1;
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
        if input[i] == 0x1b && i + 1 < input.len() && input[i + 1] == b'[' {
            // Try to match ConPTY-specific sequences
            if let Some(seq_len) = scan_conpty_sequence(&input[i..]) {
                trace!(
                    "Filtered ConPTY stdout sequence: {:02x?}",
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
/// cancelled when the remote process exits. Focus events are filtered out
/// before forwarding — see [`filter_terminal_responses`].
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
/// Focus events are filtered out before forwarding.
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

/// Forwards remote stdout to local stdout, filtering ConPTY-specific sequences.
///
/// Reads chunks from the remote process stdout, strips sequences that would
/// confuse the local terminal (Win32 input mode, focus tracking, XTWINOPS resize),
/// and writes the filtered output to local stdout.
async fn forward_stdout(mut reader: distant_core::RemoteStdout) -> std::io::Result<()> {
    use std::io::Write;

    let mut filtered = Vec::with_capacity(8192);
    loop {
        match reader.read().await {
            Ok(buf) if buf.is_empty() => break,
            Ok(buf) => {
                trace!(
                    "stdout: {} bytes: {:02x?}",
                    buf.len(),
                    &buf[..buf.len().min(64)]
                );
                filtered.clear();
                filter_conpty_stdout(&buf, &mut filtered);
                if !filtered.is_empty() {
                    let mut stdout = std::io::stdout().lock();
                    stdout.write_all(&filtered)?;
                    stdout.flush()?;
                }
            }
            Err(_) => break,
        }
    }
    Ok(())
}

/// Forwards remote stderr to local stderr without filtering.
async fn forward_stderr(mut reader: distant_core::RemoteStderr) -> std::io::Result<()> {
    use std::io::Write;

    loop {
        match reader.read().await {
            Ok(buf) if buf.is_empty() => break,
            Ok(buf) => {
                trace!(
                    "stderr: {} bytes: {:02x?}",
                    buf.len(),
                    &buf[..buf.len().min(64)]
                );
                let mut stderr = std::io::stderr().lock();
                stderr.write_all(&buf)?;
                stderr.flush()?;
            }
            Err(_) => break,
        }
    }
    Ok(())
}

/// Maximum time to wait for stdout/stderr to drain after the remote process exits.
const OUTPUT_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

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
        _max_chunk_size: usize,
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

        // Forward remote stdout (with ConPTY filtering) and stderr to local terminal.
        // This replaces RemoteProcessLink for the shell to allow stdout filtering.
        let stdout_task = tokio::spawn(forward_stdout(proc.stdout.take().unwrap()));
        let stderr_task = tokio::spawn(forward_stderr(proc.stderr.take().unwrap()));

        let status = proc.wait().await.context("Failed to wait for process")?;

        // Abort background tasks so the process can exit cleanly
        stdin_task.abort();
        resize_task.abort();

        // Drain stdout/stderr with timeout (matches link.rs OUTPUT_DRAIN_TIMEOUT)
        let drain = async {
            let _ = stdout_task.await;
            let _ = stderr_task.await;
        };
        if tokio::time::timeout(OUTPUT_DRAIN_TIMEOUT, drain)
            .await
            .is_err()
        {
            warn!(
                "stdout/stderr drain timed out after {}s",
                OUTPUT_DRAIN_TIMEOUT.as_secs()
            );
        }

        // Reset terminal state that ConPTY may have enabled before dropping raw mode
        {
            use std::io::Write;
            let _ = std::io::stdout().write_all(b"\x1b[?1004l");
            let _ = std::io::stdout().flush();
        }

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
    fn filter_passes_through_csi_sequences_that_are_not_responses() {
        // SGR (color): ESC [ 3 1 m — not a response
        let input = b"\x1b[31m";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    #[test]
    fn filter_passes_through_terminal_query_responses() {
        // DSR response: ESC [ 24 ; 80 R — must be forwarded to remote process
        let dsr = b"\x1b[24;80R";
        let mut out = Vec::new();
        filter_terminal_responses(dsr, &mut out);
        assert_eq!(out, dsr.to_vec(), "DSR should pass through");

        // DA1 response: ESC [ ? 64 ; 1 ; 2 c
        let da1 = b"\x1b[?64;1;2c";
        out.clear();
        filter_terminal_responses(da1, &mut out);
        assert_eq!(out, da1.to_vec(), "DA1 should pass through");

        // DA2 response: ESC [ > 1 ; 10 ; 0 c
        let da2 = b"\x1b[>1;10;0c";
        out.clear();
        filter_terminal_responses(da2, &mut out);
        assert_eq!(out, da2.to_vec(), "DA2 should pass through");
    }

    // ─── focus event filtering tests ───

    #[test]
    fn filter_strips_focus_in_event() {
        let input = b"\x1b[I";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert!(
            out.is_empty(),
            "Focus in event should be filtered: {:?}",
            out
        );
    }

    #[test]
    fn filter_strips_focus_out_event() {
        let input = b"\x1b[O";
        let mut out = Vec::new();
        filter_terminal_responses(input, &mut out);
        assert!(
            out.is_empty(),
            "Focus out event should be filtered: {:?}",
            out
        );
    }

    #[test]
    fn filter_strips_focus_events_mixed_with_text() {
        let mut input = Vec::new();
        input.extend_from_slice(b"hello");
        input.extend_from_slice(b"\x1b[I");
        input.extend_from_slice(b"world");
        input.extend_from_slice(b"\x1b[O");
        input.extend_from_slice(b"!");

        let mut out = Vec::new();
        filter_terminal_responses(&input, &mut out);
        assert_eq!(out, b"helloworld!");
    }

    // ─── filter_conpty_stdout tests ───

    #[test]
    fn filter_conpty_strips_win32_input_mode_enable() {
        let input = b"\x1b[?9001h";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(
            out.is_empty(),
            "Win32 input mode enable should be stripped: {:?}",
            out
        );
    }

    #[test]
    fn filter_conpty_strips_win32_input_mode_disable() {
        let input = b"\x1b[?9001l";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(
            out.is_empty(),
            "Win32 input mode disable should be stripped: {:?}",
            out
        );
    }

    #[test]
    fn filter_conpty_strips_focus_tracking_enable() {
        let input = b"\x1b[?1004h";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(
            out.is_empty(),
            "Focus tracking enable should be stripped: {:?}",
            out
        );
    }

    #[test]
    fn filter_conpty_strips_focus_tracking_disable() {
        let input = b"\x1b[?1004l";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(
            out.is_empty(),
            "Focus tracking disable should be stripped: {:?}",
            out
        );
    }

    #[test]
    fn filter_conpty_strips_xtwinops_resize() {
        let input = b"\x1b[8;35;130t";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert!(
            out.is_empty(),
            "XTWINOPS resize should be stripped: {:?}",
            out
        );
    }

    #[test]
    fn filter_conpty_passes_through_normal_content() {
        let input = b"hello world\r\n";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    #[test]
    fn filter_conpty_passes_through_standard_modes() {
        // ESC[?25h (show cursor) should NOT be stripped
        let input = b"\x1b[?25h";
        let mut out = Vec::new();
        filter_conpty_stdout(input, &mut out);
        assert_eq!(
            out,
            input.to_vec(),
            "Standard DEC modes should pass through"
        );
    }

    #[test]
    fn filter_conpty_preserves_content_around_stripped_sequences() {
        let mut input = Vec::new();
        input.extend_from_slice(b"before");
        input.extend_from_slice(b"\x1b[?9001h");
        input.extend_from_slice(b"middle");
        input.extend_from_slice(b"\x1b[?1004h");
        input.extend_from_slice(b"after");
        input.extend_from_slice(b"\x1b[8;35;130t");

        let mut out = Vec::new();
        filter_conpty_stdout(&input, &mut out);
        assert_eq!(out, b"beforemiddleafter");
    }
}
