//! Cross-platform PTY session management for integration tests.
//!
//! Provides [`PtySession`] which wraps `portable-pty` with expect-like matching
//! for test assertions. Used by shell and spawn PTY tests that require a real
//! terminal (PTY allocation, raw mode, etc.).
//!
//! # How PTY tests work
//!
//! Tests spawn a `distant shell` or `distant spawn --pty` command inside a real
//! PTY via [`PtySession::spawn`]. A background reader thread accumulates all
//! output from the PTY master into a shared buffer. Tests then:
//!
//! 1. **Send input** via [`PtySession::send`] / [`PtySession::send_line`] —
//!    bytes are written to the PTY master, which forwards them to the child's
//!    stdin as if a user typed them.
//! 2. **Assert output** via [`PtySession::expect`] — polls the accumulated
//!    buffer for a substring within a configurable timeout. This proves that the
//!    distant CLI correctly relayed the input to the remote process and returned
//!    the output through the PTY.
//! 3. **Verify exit** via [`PtySession::wait_for_exit`] — confirms the child
//!    exited with the expected status code.
//!
//! Helper binaries (`pty-echo`, `pty-interactive`, `pty-password`) run on the
//! remote side to exercise specific PTY scenarios: raw echo, interactive prompts,
//! and password input with echo disabled.
//!
//! On Windows, ConPTY cursor position queries (`\x1b[6n`) are handled
//! automatically by the reader thread to prevent child I/O deadlocks.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{
    Child as PortablePtyChild, CommandBuilder, MasterPty, PtySize, native_pty_system,
};

/// Default timeout for `expect()` calls waiting for PTY output.
const EXPECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum time to wait for a child process to exit.
const EXIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Default PTY column count.
const PTY_COLS: u16 = 120;

/// Default PTY row count.
const PTY_ROWS: u16 = 40;

/// Polling interval for busy-wait loops checking PTY output.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Polling interval for the exit-wait loop (shorter since we are just
/// checking process status, not scanning a buffer).
const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Cross-platform PTY session for testing.
///
/// Wraps `portable-pty` with expect-like matching for test assertions.
/// Spawns a reader thread to accumulate output, enabling non-blocking
/// `expect()` calls with configurable timeout.
pub struct PtySession {
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
            std::thread::sleep(POLL_INTERVAL);
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
            std::thread::sleep(EXIT_POLL_INTERVAL);
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
