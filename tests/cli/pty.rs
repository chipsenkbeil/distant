//! Integration tests for PTY and predictive echo features.
//!
//! Tests the `distant shell` and `distant spawn --pty` commands with
//! purpose-built helper binaries (`pty-echo`, `pty-interactive`) that exercise
//! PTY I/O, prompt handling, and exit behavior.
//!
//! Uses `portable-pty` for cross-platform PTY session management. On Windows,
//! ConPTY cursor position queries (`\x1b[6n`) are handled automatically by the
//! reader thread to prevent child I/O deadlocks.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use distant_test_harness::exe;
use distant_test_harness::manager::ManagerCtx;

/// Cross-platform PTY session for testing.
///
/// Wraps `portable-pty` with expect-like matching for test assertions.
/// Spawns a reader thread to accumulate output, enabling non-blocking
/// `expect()` calls with configurable timeout.
pub(super) struct PtySession {
    #[allow(dead_code)]
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    buffer: Arc<Mutex<Vec<u8>>>,
    timeout: Duration,
    last_match_end: usize,
}

impl PtySession {
    /// Spawns a command in a new PTY and starts a background reader thread.
    pub fn spawn(program: &PathBuf, args: &[String]) -> Self {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("Failed to open PTY pair");

        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);

        let child = pair
            .slave
            .spawn_command(cmd)
            .expect("Failed to spawn command in PTY");
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .expect("Failed to clone PTY reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(
            pair.master
                .take_writer()
                .expect("Failed to take PTY writer"),
        ));

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let buf_clone = Arc::clone(&buffer);

        // On Windows, the reader thread needs writer access to respond
        // to ConPTY cursor position queries.
        #[cfg(windows)]
        let writer_clone = Arc::clone(&writer);

        std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            #[cfg(windows)]
            let mut pending = Vec::new();

            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        #[cfg(windows)]
                        {
                            pending.extend_from_slice(&chunk[..n]);

                            // Handle ConPTY cursor position query (\x1b[6n).
                            // ConPTY with PSEUDOCONSOLE_INHERIT_CURSOR blocks
                            // all child I/O until it receives a cursor position
                            // response (\x1b[row;colR).
                            while let Some(pos) = find_subsequence_from(&pending, b"\x1b[6n", 0) {
                                if let Ok(mut w) = writer_clone.lock() {
                                    let _ = w.write_all(b"\x1b[1;1R");
                                    let _ = w.flush();
                                }
                                pending.drain(pos..pos + 4);
                            }
                            buf_clone.lock().unwrap().extend_from_slice(&pending);
                            pending.clear();
                        }

                        #[cfg(not(windows))]
                        {
                            buf_clone.lock().unwrap().extend_from_slice(&chunk[..n]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        PtySession {
            master: pair.master,
            writer,
            child,
            buffer,
            timeout: Duration::from_secs(30),
            last_match_end: 0,
        }
    }

    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    pub fn send(&mut self, text: &str) {
        let mut w = self.writer.lock().unwrap();
        w.write_all(text.as_bytes())
            .expect("Failed to write to PTY");
        w.flush().ok();
    }

    pub fn send_line(&mut self, text: &str) {
        self.send(&format!("{text}\n"));
    }

    /// Waits for `needle` to appear in PTY output after the last match position.
    pub fn expect(&mut self, needle: &str) {
        let needle_bytes = needle.as_bytes();
        let deadline = Instant::now() + self.timeout;
        loop {
            {
                let buf = self.buffer.lock().unwrap();
                if let Some(pos) = find_subsequence_from(&buf, needle_bytes, self.last_match_end) {
                    self.last_match_end = pos + needle_bytes.len();
                    return;
                }
            }
            assert!(
                Instant::now() < deadline,
                "Timed out waiting for '{needle}' in PTY output. Buffer: {:?}",
                String::from_utf8_lossy(&self.buffer.lock().unwrap())
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("Failed to resize PTY");
    }

    pub fn wait_for_exit(&mut self) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.exit_code(),
                Ok(None) => {}
                Err(e) => panic!("Error waiting for process: {e}"),
            }
            assert!(Instant::now() < deadline, "Process did not exit within 60s");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Finds `needle` in `haystack` starting from byte offset `start`.
fn find_subsequence_from(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if start >= haystack.len() || needle.is_empty() {
        return None;
    }
    haystack[start..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + start)
}

/// Builds cmd_parts for `distant shell` with extra args.
fn shell_cmd_args(ctx: &ManagerCtx, extra_args: &[&str]) -> (PathBuf, Vec<String>) {
    let (bin, mut args) = ctx.cmd_parts(["shell"]);
    for arg in extra_args {
        args.push(arg.to_string());
    }
    (bin, args)
}

/// Builds cmd_parts for `distant spawn` with extra args.
fn spawn_cmd_args(ctx: &ManagerCtx, extra_args: &[&str]) -> (PathBuf, Vec<String>) {
    let (bin, mut args) = ctx.cmd_parts(["spawn"]);
    for arg in extra_args {
        args.push(arg.to_string());
    }
    (bin, args)
}

#[tokio::test]
async fn shell_pty_echo_roundtrip() {
    let ctx = ManagerCtx::start();
    let pty_echo = exe::build_pty_echo()
        .await
        .expect("Failed to build pty-echo");
    let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

    let (bin, args) = shell_cmd_args(&ctx, &["--", pty_echo_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.send("abc");
    session.expect("abc");
}

#[tokio::test]
async fn shell_pty_interactive_prompt() {
    let ctx = ManagerCtx::start();
    let pty_interactive = exe::build_pty_interactive()
        .await
        .expect("Failed to build pty-interactive");
    let pty_interactive_str = pty_interactive
        .to_str()
        .expect("pty-interactive path is not valid UTF-8");

    let (bin, args) = shell_cmd_args(&ctx, &["--", pty_interactive_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("$ ");
}

#[tokio::test]
async fn shell_pty_interactive_exit() {
    let ctx = ManagerCtx::start();
    let pty_interactive = exe::build_pty_interactive()
        .await
        .expect("Failed to build pty-interactive");
    let pty_interactive_str = pty_interactive
        .to_str()
        .expect("pty-interactive path is not valid UTF-8");

    let (bin, args) = shell_cmd_args(&ctx, &["--", pty_interactive_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("$ ");

    // Send Ctrl+D (EOF). The server-side PTY line discipline interprets
    // 0x04 as EOF when the line buffer is empty, causing pty-interactive's
    // BufRead::lines() iterator to return None and exit cleanly.
    for _ in 0..5 {
        session.send("\x04");
        std::thread::sleep(Duration::from_millis(300));
        if !session.is_alive() {
            break;
        }
    }

    let exit_code = session.wait_for_exit();
    assert_eq!(exit_code, 0, "Expected exit code 0");
}

#[tokio::test]
async fn spawn_pty_flag() {
    let ctx = ManagerCtx::start();
    let pty_echo = exe::build_pty_echo()
        .await
        .expect("Failed to build pty-echo");
    let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

    let (bin, args) = spawn_cmd_args(&ctx, &["--pty", "--", pty_echo_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.send("hello");
    session.expect("hello");
}

#[tokio::test]
async fn predict_off_runs_command() {
    let ctx = ManagerCtx::start();

    // On Windows, `echo` is a cmd.exe built-in (no echo.exe), so we wrap it.
    #[cfg(unix)]
    let extra_args: &[&str] = &["--predict", "off", "--", "echo", "predict-off-ok"];
    #[cfg(windows)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "cmd",
        "/c",
        "echo",
        "predict-off-ok",
    ];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("predict-off-ok");
    let exit_code = session.wait_for_exit();
    assert_eq!(exit_code, 0, "Expected exit code 0 with predict off");
}

#[tokio::test]
async fn shell_pty_ctrl_c() {
    let ctx = ManagerCtx::start();
    let pty_interactive = exe::build_pty_interactive()
        .await
        .expect("Failed to build pty-interactive");
    let pty_interactive_str = pty_interactive
        .to_str()
        .expect("pty-interactive path is not valid UTF-8");

    let (bin, args) = shell_cmd_args(&ctx, &["--", pty_interactive_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("$ ");

    // Send Ctrl+C (ETX byte 0x03). The server-side PTY line discipline
    // translates this to SIGINT for the pty-interactive process group.
    session.send("\x03");
    std::thread::sleep(Duration::from_millis(200));
    session.send_line("");

    // After Ctrl+C handling, pty-interactive prints a new `$ `.
    session.expect("$ ");
}

#[tokio::test]
async fn predict_on_password_suppressed() {
    let ctx = ManagerCtx::start();
    let pty_password = exe::build_pty_password()
        .await
        .expect("Failed to build pty-password");
    let pty_password_str = pty_password
        .to_str()
        .expect("pty-password path is not valid UTF-8");

    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "on", "--", pty_password_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("Password: ");
    session.send_line("secret123");
    session.expect("Authenticated.");
}

#[tokio::test]
async fn predict_on_runs_command() {
    let ctx = ManagerCtx::start();

    #[cfg(unix)]
    let extra_args: &[&str] = &["--predict", "on", "--", "echo", "predict-on-ok"];
    #[cfg(windows)]
    let extra_args: &[&str] = &[
        "--predict",
        "on",
        "--",
        "cmd",
        "/c",
        "echo",
        "predict-on-ok",
    ];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("predict-on-ok");
    let exit_code = session.wait_for_exit();
    assert_eq!(exit_code, 0, "Expected exit code 0 with predict on");
}

#[tokio::test]
async fn predict_off_no_local_echo() {
    let ctx = ManagerCtx::start();
    let pty_password = exe::build_pty_password()
        .await
        .expect("Failed to build pty-password");
    let pty_password_str = pty_password
        .to_str()
        .expect("pty-password path is not valid UTF-8");

    // With --predict off, the prediction engine is completely disabled.
    // Running pty-password verifies that password entry works correctly
    // without predictions interfering.
    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "off", "--", pty_password_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("Password: ");
    session.send_line("secret123");
    session.expect("Authenticated.");
}

#[tokio::test]
async fn predict_off_server_echo_only() {
    let ctx = ManagerCtx::start();
    let pty_echo = exe::build_pty_echo()
        .await
        .expect("Failed to build pty-echo");
    let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

    // With --predict off, all echo must come from the server.
    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "off", "--", pty_echo_str]);
    let mut session = PtySession::spawn(&bin, &args);
    session.set_timeout(Duration::from_secs(60));

    // Wait for pty-echo to start
    std::thread::sleep(Duration::from_secs(1));

    session.send("xyz");
    session.expect("xyz");
}

#[tokio::test]
async fn predict_on_immediate_echo() {
    let ctx = ManagerCtx::start();
    let pty_echo = exe::build_pty_echo()
        .await
        .expect("Failed to build pty-echo");
    let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

    // With --predict on, the prediction engine echoes characters locally
    // before server confirmation.
    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "on", "--", pty_echo_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.send("predict-immediate");
    session.expect("predict-immediate");
}

#[tokio::test]
async fn predict_on_mismatch_correction() {
    let ctx = ManagerCtx::start();
    let pty_password = exe::build_pty_password()
        .await
        .expect("Failed to build pty-password");
    let pty_password_str = pty_password
        .to_str()
        .expect("pty-password path is not valid UTF-8");

    // Tests the full mismatch detection → suppression → correction cycle:
    // predict on → password prompt disables echo → mismatch detected →
    // predictions suppressed → password transmitted → echo resumes.
    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "on", "--", pty_password_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("Password: ");
    session.send_line("secret123");
    session.expect("Authenticated.");
}

#[tokio::test]
async fn spawn_pty_resize() {
    let ctx = ManagerCtx::start();

    // Platform-specific commands to check terminal size after a delay.
    // The delay gives us time to resize the PTY before the command checks.
    #[cfg(unix)]
    let extra_args: &[&str] = &["--predict", "off", "--", "sh", "-c", "'sleep 2; stty size'"];

    // On Windows, `mode con` reports console dimensions including "Lines:" and "Columns:".
    // Each token is a separate arg so cmd.exe interprets `&` and `>` as operators.
    #[cfg(windows)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "cmd",
        "/c",
        "timeout",
        "/t",
        "2",
        "/nobreak",
        ">nul",
        "2>nul",
        "&",
        "mode",
        "con",
    ];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = PtySession::spawn(&bin, &args);

    // Resize PTY to 50 rows x 132 cols before the delay expires.
    session.resize(50, 132);

    // After delay, the command reports terminal size — should show 50.
    session.expect("50");
}

#[tokio::test]
async fn shell_alternate_screen_entry() {
    let ctx = ManagerCtx::start();

    // Verify that alternate screen mode entry/exit works through distant shell.
    // Text inside the alternate screen is discarded on rmcup, so the marker
    // is printed AFTER rmcup.
    #[cfg(unix)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "sh",
        "-c",
        "'tput smcup 2>/dev/null; tput rmcup 2>/dev/null; echo ALT_ENTRY_OK'",
    ];

    // On Windows, use PowerShell to emit ANSI escape sequences directly.
    // [char]27 produces the ESC character for VT sequences.
    #[cfg(windows)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "powershell",
        "-NoProfile",
        "-Command",
        "\"Write-Host -NoNewline ([char]27+'[?1049h'); Write-Host -NoNewline ([char]27+'[?1049l'); Write-Host 'ALT_ENTRY_OK'\"",
    ];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("ALT_ENTRY_OK");
}

#[tokio::test]
async fn shell_alternate_screen_exit() {
    let ctx = ManagerCtx::start();

    // Enter alt screen, print content (discarded on rmcup), exit alt screen,
    // then print a marker in the main buffer.
    #[cfg(unix)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "sh",
        "-c",
        "'tput smcup 2>/dev/null; echo IN_ALT; tput rmcup 2>/dev/null; echo RESTORED'",
    ];

    #[cfg(windows)]
    let extra_args: &[&str] = &[
        "--predict",
        "off",
        "--",
        "powershell",
        "-NoProfile",
        "-Command",
        "\"Write-Host -NoNewline ([char]27+'[?1049h'); Write-Host 'IN_ALT'; Write-Host -NoNewline ([char]27+'[?1049l'); Write-Host 'RESTORED'\"",
    ];

    let (bin, args) = shell_cmd_args(&ctx, extra_args);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("RESTORED");
}
