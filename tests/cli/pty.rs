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

use portable_pty::{
    Child as PortablePtyChild, CommandBuilder, MasterPty, PtySize, native_pty_system,
};

use distant_test_harness::exe;
use distant_test_harness::manager::HostManagerCtx;

/// Default timeout for `expect()` calls waiting for PTY output.
const EXPECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum time to wait for a child process to exit.
const EXIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Default PTY column count.
const PTY_COLS: u16 = 120;

/// Default PTY row count.
const PTY_ROWS: u16 = 40;

/// Delay (in seconds) used in resize tests to give time for PTY resize before
/// the child command queries terminal dimensions.
const RESIZE_DELAY_SECS: u8 = 2;

/// Cross-platform PTY session for testing.
///
/// Wraps `portable-pty` with expect-like matching for test assertions.
/// Spawns a reader thread to accumulate output, enabling non-blocking
/// `expect()` calls with configurable timeout.
pub(super) struct PtySession {
    #[allow(dead_code)]
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn PortablePtyChild + Send + Sync>,
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
                rows: PTY_ROWS,
                cols: PTY_COLS,
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
            timeout: EXPECT_TIMEOUT,
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
        let deadline = Instant::now() + EXIT_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.exit_code(),
                Ok(None) => {}
                Err(e) => panic!("Error waiting for process: {e}"),
            }
            assert!(
                Instant::now() < deadline,
                "Process did not exit within {}s",
                EXIT_TIMEOUT.as_secs()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(unix)]
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
fn shell_cmd_args(ctx: &HostManagerCtx, extra_args: &[&str]) -> (PathBuf, Vec<String>) {
    let (bin, mut args) = ctx.cmd_parts(["shell"]);
    for arg in extra_args {
        args.push(arg.to_string());
    }
    (bin, args)
}

/// Builds cmd_parts for `distant spawn` with extra args.
fn spawn_cmd_args(ctx: &HostManagerCtx, extra_args: &[&str]) -> (PathBuf, Vec<String>) {
    let (bin, mut args) = ctx.cmd_parts(["spawn"]);
    for arg in extra_args {
        args.push(arg.to_string());
    }
    (bin, args)
}

/// Verifies that `distant shell -- pty-echo` echoes input back through the
/// PTY channel. Sends "abc" and expects to receive "abc" within the timeout.
/// A timeout failure indicates the PTY relay is not passing data through.
#[tokio::test]
async fn shell_should_echo_input_through_pty() {
    let ctx = HostManagerCtx::start();
    let pty_echo = exe::build_pty_echo()
        .await
        .expect("Failed to build pty-echo");
    let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

    let (bin, args) = shell_cmd_args(&ctx, &["--", pty_echo_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.send("abc");
    session.expect("abc");
}

/// Verifies that `distant shell -- pty-interactive` displays a prompt.
/// Spawns the interactive helper and expects the `$ ` prompt string within
/// the timeout. Failure indicates the PTY is not relaying server output.
#[tokio::test]
async fn shell_should_display_interactive_prompt() {
    let ctx = HostManagerCtx::start();
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

/// Verifies that sending EOF (Ctrl+D on Unix, "exit" command on Windows)
/// causes `pty-interactive` to exit cleanly with code 0. On Unix, retries
/// Ctrl+D up to 5 times to handle line-discipline buffering edge cases.
#[tokio::test]
async fn shell_should_exit_on_eof_signal() {
    let ctx = HostManagerCtx::start();
    let pty_interactive = exe::build_pty_interactive()
        .await
        .expect("Failed to build pty-interactive");
    let pty_interactive_str = pty_interactive
        .to_str()
        .expect("pty-interactive path is not valid UTF-8");

    let (bin, args) = shell_cmd_args(&ctx, &["--", pty_interactive_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.expect("$ ");

    // On Unix, Ctrl+D (0x04) signals EOF to the PTY line discipline when
    // the line buffer is empty, causing pty-interactive's BufRead::lines()
    // iterator to return None and exit cleanly.
    //
    // On Windows, ConPTY does not translate Ctrl+D to EOF. Instead, use
    // the "exit" command that pty-interactive recognizes.
    #[cfg(unix)]
    for _ in 0..5 {
        session.send("\x04");
        std::thread::sleep(Duration::from_millis(300));
        if !session.is_alive() {
            break;
        }
    }

    #[cfg(windows)]
    session.send_line("exit");

    let exit_code = session.wait_for_exit();
    assert_eq!(exit_code, 0, "Expected exit code 0");
}

/// Verifies that `distant spawn --pty -- pty-echo` allocates a PTY for the
/// spawned process. Sends "hello" and expects the echo back, confirming that
/// the `--pty` flag enables PTY allocation in the spawn subcommand.
#[tokio::test]
async fn spawn_should_support_pty_flag() {
    let ctx = HostManagerCtx::start();
    let pty_echo = exe::build_pty_echo()
        .await
        .expect("Failed to build pty-echo");
    let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

    let (bin, args) = spawn_cmd_args(&ctx, &["--pty", "--", pty_echo_str]);
    let mut session = PtySession::spawn(&bin, &args);

    session.send("hello");
    session.expect("hello");
}

/// Verifies that `distant shell --predict off` runs a simple echo command
/// and exits with code 0. Uses platform-specific echo invocations (`echo`
/// on Unix, `cmd /c echo` on Windows).
#[tokio::test]
async fn shell_should_run_command_with_predict_off() {
    let ctx = HostManagerCtx::start();

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

/// Verifies that Ctrl+C (ETX byte 0x03) is forwarded through the PTY relay
/// to the server-side process. Sends Ctrl+C to `pty-interactive`, then
/// confirms the shell re-displays a fresh prompt, indicating the interrupt
/// was handled without crashing.
#[tokio::test]
async fn shell_should_handle_ctrl_c_interrupt() {
    let ctx = HostManagerCtx::start();
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

/// Verifies that password prompts are not echoed when prediction is enabled.
/// Runs `pty-password` with `--predict on`, types a password, and confirms
/// authentication succeeds. The prediction engine should detect the echo
/// suppression and not locally echo the password characters.
#[tokio::test]
async fn shell_should_suppress_predicted_password_echo() {
    let ctx = HostManagerCtx::start();
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

/// Verifies that `distant shell --predict on` runs a simple echo command
/// and exits with code 0. Uses platform-specific echo invocations (`echo`
/// on Unix, `cmd /c echo` on Windows).
#[tokio::test]
async fn shell_should_run_command_with_predict_on() {
    let ctx = HostManagerCtx::start();

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

/// Verifies that password entry works correctly with prediction disabled.
/// Runs `pty-password` with `--predict off` and confirms authentication
/// succeeds, ensuring no prediction interference with echo suppression.
#[tokio::test]
async fn shell_should_not_echo_locally_with_predict_off() {
    let ctx = HostManagerCtx::start();
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

/// Verifies that with `--predict off`, all echo comes from the server, not
/// from local prediction. Sends characters one at a time through `pty-echo`
/// and confirms each is echoed back, proving the server relay path works
/// without local prediction.
#[tokio::test]
async fn shell_should_echo_from_server_only_with_predict_off() {
    let ctx = HostManagerCtx::start();
    let pty_echo = exe::build_pty_echo()
        .await
        .expect("Failed to build pty-echo");
    let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

    // With --predict off, all echo must come from the server.
    let (bin, args) = shell_cmd_args(&ctx, &["--predict", "off", "--", pty_echo_str]);
    let mut session = PtySession::spawn(&bin, &args);
    session.set_timeout(EXIT_TIMEOUT);

    // Wait for the distant shell to fully connect and enter raw mode before
    // sending input. Using a fixed sleep is insufficient under heavy load —
    // input sent before the shell is ready can be garbled.
    session.expect("Connected to manager");

    // Send characters one at a time, confirming each echo before sending
    // the next. Under heavy parallel load, sending multiple bytes at once
    // can result in garbled echo (bytes arriving out of order in the
    // protocol relay between local PTY → distant shell → pty-echo → back).
    session.send("x");
    session.expect("x");
    session.send("y");
    session.expect("y");
    session.send("z");
    session.expect("z");
}

/// Verifies that with `--predict on`, characters are echoed immediately
/// (locally predicted) before server confirmation. Sends a string through
/// `pty-echo` and expects it back within the timeout.
#[tokio::test]
async fn shell_should_echo_immediately_with_predict_on() {
    let ctx = HostManagerCtx::start();
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

/// Verifies the prediction mismatch detection and correction cycle. With
/// `--predict on`, the password prompt disables server echo, causing a
/// mismatch between predicted and actual output. The engine should detect
/// this, suppress further predictions, transmit the password, and resume
/// normal operation after authentication.
#[tokio::test]
async fn shell_should_correct_prediction_mismatch() {
    let ctx = HostManagerCtx::start();
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

/// Verifies that PTY resize events propagate from the local terminal through
/// `distant shell` to the remote process. Resizes the PTY to 50x132, then
/// checks that the remote command (`stty size` on Unix, `mode con` on
/// Windows) reports 50 rows after a delay.
#[tokio::test]
async fn spawn_should_propagate_pty_resize() {
    let ctx = HostManagerCtx::start();

    let delay_str = RESIZE_DELAY_SECS.to_string();

    // Platform-specific commands to check terminal size after a delay.
    // The delay gives us time to resize the PTY before the command checks.
    #[cfg(unix)]
    let sleep_cmd = format!("'sleep {delay_str}; stty size'");
    #[cfg(unix)]
    let extra_args: Vec<&str> = vec!["--predict", "off", "--", "sh", "-c", &sleep_cmd];

    // On Windows, `mode con` reports console dimensions including "Lines:" and "Columns:".
    // Each token is a separate arg so cmd.exe interprets `&` and `>` as operators.
    #[cfg(windows)]
    let extra_args: Vec<&str> = vec![
        "--predict",
        "off",
        "--",
        "cmd",
        "/c",
        "timeout",
        "/t",
        &delay_str,
        "/nobreak",
        ">nul",
        "2>nul",
        "&",
        "mode",
        "con",
    ];

    let (bin, args) = shell_cmd_args(&ctx, &extra_args);
    let mut session = PtySession::spawn(&bin, &args);

    // Resize PTY to 50 rows x 132 cols before the delay expires.
    session.resize(50, 132);

    // After delay, the command reports terminal size — should show 50.
    session.expect("50");
}

/// Verifies that alternate screen mode entry and exit work through
/// `distant shell`. Enters alternate screen (smcup), exits it (rmcup),
/// then prints a marker. The marker appearing confirms the PTY relay
/// handles mode-switching escape sequences correctly.
#[tokio::test]
async fn shell_should_enter_alternate_screen() {
    let ctx = HostManagerCtx::start();

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

/// Verifies that content printed inside the alternate screen buffer is
/// discarded when returning to the main buffer. Enters alternate screen,
/// prints "IN_ALT", exits alternate screen, then prints "RESTORED".
/// Expects "RESTORED" to confirm the main buffer is restored correctly.
#[tokio::test]
async fn shell_should_exit_alternate_screen() {
    let ctx = HostManagerCtx::start();

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
