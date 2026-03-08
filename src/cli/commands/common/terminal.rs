use std::time::Duration;

use anyhow::Context;
use distant_core::protocol::PtySize;
use distant_core::{RemoteProcess, RemoteProcessResizer, RemoteStdin};
use log::*;
use termwiz::caps::Capabilities;
use termwiz::input::{InputEvent, KeyCodeEncodeModes, KeyboardEncoding};
use termwiz::terminal::{Terminal, new_terminal};
use tokio::task::JoinHandle;

use super::RemoteProcessLink;

/// Configures which terminal escape sequences to strip from remote output.
///
/// When connecting to a remote host (especially Windows with ConPTY), the remote
/// terminal may emit escape sequences that cause problems when forwarded to the
/// local terminal emulator. This struct defines a set of DEC private modes to
/// strip from output, preventing issues like:
///
/// - Mouse tracking sequences being echoed as raw text
/// - Focus tracking events generating noise
/// - Window resize sequences causing feedback loops
///
/// # Examples
///
/// ```ignore
/// use distant::cli::commands::common::terminal::TerminalSanitizer;
///
/// let mut out = Vec::new();
/// TerminalSanitizer::CONPTY.filter(b"\x1b[?1006hHello", &mut out);
/// assert_eq!(out, b"Hello");
/// ```
pub struct TerminalSanitizer {
    /// DEC private modes to strip (both `h` set and `l` reset variants).
    blocked_modes: &'static [u32],
    /// Whether to strip XTWINOPS resize sequences (`ESC[8;<rows>;<cols>t`).
    strip_xtwinops: bool,
}

impl TerminalSanitizer {
    /// Standard sanitizer for SSH shell sessions targeting ConPTY hosts.
    ///
    /// Strips Win32 input mode, focus tracking, all mouse tracking modes,
    /// and XTWINOPS resize sequences.
    pub const CONPTY: Self = Self {
        blocked_modes: &[
            9001, // Win32 input mode
            1004, // Focus tracking
            1000, // Normal mouse tracking
            1002, // Button-event mouse tracking
            1003, // Any-event mouse tracking
            1005, // UTF-8 mouse mode
            1006, // SGR extended mouse mode
            1015, // URXVT mouse mode
        ],
        strip_xtwinops: true,
    };

    /// Filter remote output bytes, stripping blocked sequences.
    ///
    /// Scans `input` for escape sequences matching blocked DEC private modes
    /// and XTWINOPS resize sequences. All other bytes (including standard
    /// CSI sequences like SGR colors and cursor movement) are passed through
    /// unchanged into `out`.
    pub fn filter(&self, input: &[u8], out: &mut Vec<u8>) {
        let mut i = 0;
        while i < input.len() {
            if input[i] == 0x1b
                && i + 1 < input.len()
                && input[i + 1] == b'['
                && let Some(seq_len) = self.scan_sequence(&input[i..])
            {
                trace!(
                    "Filtered terminal sequence: {:02x?}",
                    &input[i..i + seq_len]
                );
                i += seq_len;
                continue;
            }
            out.push(input[i]);
            i += 1;
        }
    }

    /// Generate escape sequences that disable all blocked DEC private modes.
    ///
    /// Returns a byte sequence containing `ESC[?<mode>l` for each blocked mode.
    /// Writing this to stdout on shutdown ensures the local terminal does not
    /// remain in a state where mouse tracking or other modes are active.
    pub fn reset_sequence(&self) -> Vec<u8> {
        let mut seq = Vec::new();
        for &mode in self.blocked_modes {
            seq.extend_from_slice(format!("\x1b[?{mode}l").as_bytes());
        }
        seq
    }

    /// Scan for a blocked sequence starting at `buf[0] == ESC, buf[1] == '['`.
    ///
    /// Returns the total byte length of the sequence if it matches a blocked
    /// pattern, or `None` if it should be passed through.
    fn scan_sequence(&self, buf: &[u8]) -> Option<usize> {
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
                if self.blocked_modes.contains(&mode) {
                    return Some(j + 1);
                }
            }
            return None;
        }

        // ESC[8;<digits>;<digits>t — XTWINOPS resize
        if self.strip_xtwinops && buf[2] == b'8' && buf.len() > 3 && buf[3] == b';' {
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
}

/// Manages the local terminal for an interactive remote shell session.
///
/// Handles termwiz raw mode setup, input forwarding (keys to remote stdin,
/// resize events to remote PTY), output sanitization via [`TerminalSanitizer`],
/// and terminal cleanup on shutdown.
///
/// All three `Shell::spawn()` call sites (distant shell, distant spawn --pty,
/// distant ssh) go through this struct, ensuring consistent terminal handling.
pub struct TerminalSession {
    _input_task: JoinHandle<()>,
    link: RemoteProcessLink,
}

impl TerminalSession {
    /// Start a terminal session for the given remote process.
    ///
    /// Takes ownership of the process's stdin/stdout/stderr pipes.
    /// Sets the local terminal to raw mode via termwiz, spawns an input
    /// handler task (forwarding key events and resize events), and creates
    /// a filtered output link using [`TerminalSanitizer::CONPTY`].
    ///
    /// # Errors
    ///
    /// Returns an error if terminal capabilities cannot be loaded or the
    /// terminal cannot be created or set to raw mode.
    pub fn start(proc: &mut RemoteProcess, max_chunk_size: usize) -> anyhow::Result<Self> {
        let mut terminal = new_terminal(
            Capabilities::new_from_env().context("Failed to load terminal capabilities")?,
        )
        .context("Failed to create terminal")?;
        terminal.set_raw_mode().context("Failed to set raw mode")?;

        let mut stdin = proc.stdin.take().unwrap();
        let resizer = proc.clone_resizer();
        let input_task = tokio::spawn(async move {
            input_loop(&mut terminal, &mut stdin, resizer).await;
        });

        // Create output link with ConPTY filter.
        // The closure is non-capturing so it coerces to a fn pointer.
        let link = RemoteProcessLink::from_remote_pipes_filtered(
            None,
            proc.stdout.take().unwrap(),
            proc.stderr.take().unwrap(),
            max_chunk_size,
            |input, out| TerminalSanitizer::CONPTY.filter(input, out),
        );

        Ok(Self {
            _input_task: input_task,
            link,
        })
    }

    /// Shut down the session: drain output, then reset terminal modes.
    ///
    /// Writes reset sequences to stdout to disable any DEC private modes
    /// that may have been enabled by the remote host (mouse tracking, etc.).
    pub async fn shutdown(self) {
        self.link.shutdown().await;

        // Reset any blocked modes on the local terminal
        let reset = TerminalSanitizer::CONPTY.reset_sequence();
        if !reset.is_empty() {
            use std::io::Write;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            let _ = out.write_all(&reset);
            let _ = out.flush();
        }
    }
}

/// Input handling loop: reads terminal events and forwards them to the remote process.
async fn input_loop(
    terminal: &mut impl Terminal,
    stdin: &mut RemoteStdin,
    resizer: RemoteProcessResizer,
) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── TerminalSanitizer::filter tests ───

    #[test]
    fn filter_strips_focus_tracking_enable() {
        let input = b"\x1b[?1004h";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip ?1004h: {out:?}");
    }

    #[test]
    fn filter_strips_focus_tracking_disable() {
        let input = b"\x1b[?1004l";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip ?1004l: {out:?}");
    }

    #[test]
    fn filter_strips_win32_input_mode_enable() {
        let input = b"\x1b[?9001h";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip ?9001h: {out:?}");
    }

    #[test]
    fn filter_strips_win32_input_mode_disable() {
        let input = b"\x1b[?9001l";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip ?9001l: {out:?}");
    }

    #[test]
    fn filter_strips_xtwinops_resize() {
        let input = b"\x1b[8;24;80t";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip XTWINOPS: {out:?}");
    }

    #[test]
    fn filter_passes_through_normal_text() {
        let input = b"hello world";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    #[test]
    fn filter_passes_through_standard_csi_sequences() {
        // SGR color: ESC[1;31m
        let input = b"\x1b[1;31m";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec(), "SGR should pass through");

        // Show cursor: ESC[?25h (mode 25 is not blocked)
        let input = b"\x1b[?25h";
        out.clear();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec(), "?25h should pass through");
    }

    #[test]
    fn filter_handles_mixed_content() {
        let mut input = Vec::new();
        input.extend_from_slice(b"hello");
        input.extend_from_slice(b"\x1b[?1004h");
        input.extend_from_slice(b"world");

        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(&input, &mut out);
        assert_eq!(out, b"helloworld");
    }

    #[test]
    fn filter_handles_empty_input() {
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(b"", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn filter_handles_multiple_sequences() {
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b[?9001h");
        input.extend_from_slice(b"\x1b[?1004h");
        input.extend_from_slice(b"\x1b[8;50;120t");
        input.extend_from_slice(b"ok");

        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(&input, &mut out);
        assert_eq!(out, b"ok");
    }

    #[test]
    fn filter_passes_incomplete_esc_at_end() {
        let input = b"text\x1b";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    // ─── Mouse tracking mode tests ───

    #[test]
    fn filter_strips_normal_mouse_tracking() {
        for suffix in [b'h', b'l'] {
            let input = [0x1b, b'[', b'?', b'1', b'0', b'0', b'0', suffix];
            let mut out = Vec::new();
            TerminalSanitizer::CONPTY.filter(&input, &mut out);
            assert!(
                out.is_empty(),
                "should strip ?1000{}: {out:?}",
                suffix as char
            );
        }
    }

    #[test]
    fn filter_strips_button_event_mouse_tracking() {
        for suffix in [b'h', b'l'] {
            let input = [0x1b, b'[', b'?', b'1', b'0', b'0', b'2', suffix];
            let mut out = Vec::new();
            TerminalSanitizer::CONPTY.filter(&input, &mut out);
            assert!(
                out.is_empty(),
                "should strip ?1002{}: {out:?}",
                suffix as char
            );
        }
    }

    #[test]
    fn filter_strips_any_event_mouse_tracking() {
        for suffix in [b'h', b'l'] {
            let input = [0x1b, b'[', b'?', b'1', b'0', b'0', b'3', suffix];
            let mut out = Vec::new();
            TerminalSanitizer::CONPTY.filter(&input, &mut out);
            assert!(
                out.is_empty(),
                "should strip ?1003{}: {out:?}",
                suffix as char
            );
        }
    }

    #[test]
    fn filter_strips_utf8_mouse_mode() {
        for suffix in [b'h', b'l'] {
            let input = [0x1b, b'[', b'?', b'1', b'0', b'0', b'5', suffix];
            let mut out = Vec::new();
            TerminalSanitizer::CONPTY.filter(&input, &mut out);
            assert!(
                out.is_empty(),
                "should strip ?1005{}: {out:?}",
                suffix as char
            );
        }
    }

    #[test]
    fn filter_strips_sgr_extended_mouse_mode() {
        for suffix in [b'h', b'l'] {
            let input = [0x1b, b'[', b'?', b'1', b'0', b'0', b'6', suffix];
            let mut out = Vec::new();
            TerminalSanitizer::CONPTY.filter(&input, &mut out);
            assert!(
                out.is_empty(),
                "should strip ?1006{}: {out:?}",
                suffix as char
            );
        }
    }

    #[test]
    fn filter_strips_urxvt_mouse_mode() {
        for suffix in [b'h', b'l'] {
            let input = [0x1b, b'[', b'?', b'1', b'0', b'1', b'5', suffix];
            let mut out = Vec::new();
            TerminalSanitizer::CONPTY.filter(&input, &mut out);
            assert!(
                out.is_empty(),
                "should strip ?1015{}: {out:?}",
                suffix as char
            );
        }
    }

    #[test]
    fn filter_strips_mouse_modes_in_mixed_content() {
        let mut input = Vec::new();
        input.extend_from_slice(b"prompt$ ");
        input.extend_from_slice(b"\x1b[?1000h");
        input.extend_from_slice(b"\x1b[?1006h");
        input.extend_from_slice(b"\x1b[1;32m"); // SGR green — should pass through
        input.extend_from_slice(b"hello");

        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(&input, &mut out);
        assert_eq!(out, b"prompt$ \x1b[1;32mhello");
    }

    // ─── scan_sequence tests ───

    #[test]
    fn scan_returns_none_for_non_csi() {
        assert!(TerminalSanitizer::CONPTY.scan_sequence(b"hello").is_none());
        assert!(TerminalSanitizer::CONPTY.scan_sequence(b"\x1bO").is_none());
    }

    #[test]
    fn scan_returns_none_for_standard_private_modes() {
        // ?25h (show cursor) — not a blocked mode
        assert!(
            TerminalSanitizer::CONPTY
                .scan_sequence(b"\x1b[?25h")
                .is_none()
        );
        // ?1049h (alternate screen) — not blocked
        assert!(
            TerminalSanitizer::CONPTY
                .scan_sequence(b"\x1b[?1049h")
                .is_none()
        );
    }

    #[test]
    fn scan_returns_length_for_1004h() {
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?1004h"),
            Some(8)
        );
    }

    #[test]
    fn scan_returns_length_for_9001l() {
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?9001l"),
            Some(8)
        );
    }

    #[test]
    fn scan_returns_length_for_mouse_modes() {
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?1000h"),
            Some(8)
        );
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?1002l"),
            Some(8)
        );
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?1003h"),
            Some(8)
        );
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?1005l"),
            Some(8)
        );
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?1006h"),
            Some(8)
        );
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?1015l"),
            Some(8)
        );
    }

    #[test]
    fn scan_returns_length_for_xtwinops() {
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[8;24;80t"),
            Some(10)
        );
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[8;50;120t"),
            Some(11)
        );
    }

    #[test]
    fn scan_returns_none_for_incomplete_sequence() {
        assert!(TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[").is_none());
        assert!(TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?").is_none());
        assert!(
            TerminalSanitizer::CONPTY
                .scan_sequence(b"\x1b[?1004")
                .is_none()
        );
    }

    // ─── reset_sequence tests ───

    #[test]
    fn reset_sequence_disables_all_blocked_modes() {
        let reset = TerminalSanitizer::CONPTY.reset_sequence();
        let reset_str = String::from_utf8(reset).unwrap();

        // Should contain disable sequences for all blocked modes
        for mode in TerminalSanitizer::CONPTY.blocked_modes {
            assert!(
                reset_str.contains(&format!("\x1b[?{mode}l")),
                "reset should contain disable for mode {mode}"
            );
        }
    }

    #[test]
    fn reset_sequence_uses_l_suffix() {
        let reset = TerminalSanitizer::CONPTY.reset_sequence();
        let reset_str = String::from_utf8(reset).unwrap();

        // Should not contain any 'h' (enable) sequences
        assert!(
            !reset_str.contains('h'),
            "reset should only use 'l' (disable) suffix"
        );
    }
}
