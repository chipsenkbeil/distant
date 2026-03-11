use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use distant_core::protocol::PtySize;
use distant_core::{RemoteProcess, RemoteProcessResizer, RemoteStdin};
use log::*;
use termwiz::caps::Capabilities;
use termwiz::input::{InputEvent, KeyCodeEncodeModes, KeyboardEncoding};
use termwiz::terminal::{Terminal, new_terminal};
use tokio::task::JoinHandle;

use super::predict::{PredictMode, PredictionAction, PredictionEngine};
use super::{RemoteProcessLink, StdoutFilter};

/// Configures which terminal escape sequences to strip from remote output.
///
/// When connecting to a remote host (especially Windows with ConPTY), the remote
/// terminal may emit escape sequences that cause problems when forwarded to the
/// local terminal emulator. This struct defines DEC private modes, resize
/// sequences, and terminal queries to strip, preventing issues like:
///
/// - Mouse tracking sequences being echoed as raw text
/// - Focus tracking events generating noise
/// - Window resize sequences causing feedback loops
/// - Terminal query responses (DA, DSR, OSC color queries) polluting stdin
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
    /// Whether to strip terminal query sequences (DA1, DA2, DA3, DSR, DECRQM,
    /// XTVERSION, and OSC queries ending with `?`).
    strip_queries: bool,
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
        strip_queries: true,
    };

    /// Filter remote output bytes, stripping blocked sequences.
    ///
    /// Scans `input` for escape sequences matching blocked DEC private modes,
    /// XTWINOPS resize sequences, and terminal query sequences (when
    /// `strip_queries` is enabled). All other bytes (including standard CSI
    /// sequences like SGR colors and cursor movement) are passed through
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
            if input[i] == 0x1b
                && i + 1 < input.len()
                && input[i + 1] == b']'
                && self.strip_queries
                && let Some(seq_len) = self.scan_osc_query(&input[i..])
            {
                trace!("Filtered terminal query: {:02x?}", &input[i..i + seq_len]);
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

    /// Scan for an OSC query sequence starting at `buf[0] == ESC, buf[1] == ']'`.
    ///
    /// Matches OSC sequences whose content ends with `?` before the string
    /// terminator (either BEL `\x07` or ST `ESC \`). These are terminal queries
    /// (e.g., color queries like `\x1b]10;?\x07`) that would cause the local
    /// terminal to respond with unwanted input.
    ///
    /// Returns the total byte length of the sequence if it is a query, or `None`
    /// if it is a non-query OSC or the sequence is incomplete.
    fn scan_osc_query(&self, buf: &[u8]) -> Option<usize> {
        if buf.len() < 2 || buf[0] != 0x1b || buf[1] != b']' {
            return None;
        }

        let mut j = 2;
        while j < buf.len() {
            match buf[j] {
                // BEL terminator
                0x07 => {
                    if j > 2 && buf[j - 1] == b'?' {
                        return Some(j + 1);
                    }
                    return None;
                }
                // Possible ST terminator (ESC \)
                0x1b => {
                    if j + 1 < buf.len() && buf[j + 1] == b'\\' {
                        if j > 2 && buf[j - 1] == b'?' {
                            return Some(j + 2);
                        }
                        return None;
                    }
                    return None;
                }
                _ => j += 1,
            }
        }

        // Incomplete — no terminator found, pass through
        None
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
            // DECRQM: ESC[?<digits>$p
            if self.strip_queries && buf[j] == b'$' && j + 1 < buf.len() && buf[j + 1] == b'p' {
                return Some(j + 2);
            }
            return None;
        }

        // CSI query sequences (DA1, DA2, DA3, DSR, XTVERSION)
        if self.strip_queries {
            // DA1: ESC[c or ESC[0c
            if buf[2] == b'c' {
                return Some(3);
            }
            if buf[2] == b'0' && buf.len() > 3 && buf[3] == b'c' {
                return Some(4);
            }
            // DA2: ESC[>c or ESC[>0c
            // XTVERSION: ESC[>q or ESC[>0q
            if buf[2] == b'>' && buf.len() > 3 {
                if buf[3] == b'c' || buf[3] == b'q' {
                    return Some(4);
                }
                if buf[3] == b'0' && buf.len() > 4 && (buf[4] == b'c' || buf[4] == b'q') {
                    return Some(5);
                }
            }
            // DA3: ESC[=c
            if buf[2] == b'=' && buf.len() > 3 && buf[3] == b'c' {
                return Some(4);
            }
            // DSR: ESC[5n or ESC[6n
            if (buf[2] == b'5' || buf[2] == b'6') && buf.len() > 3 && buf[3] == b'n' {
                return Some(4);
            }
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
    pub fn start(
        proc: &mut RemoteProcess,
        max_chunk_size: usize,
        predict_mode: PredictMode,
    ) -> anyhow::Result<Self> {
        let mut terminal = new_terminal(
            Capabilities::new_from_env().context("Failed to load terminal capabilities")?,
        )
        .context("Failed to create terminal")?;
        terminal.set_raw_mode().context("Failed to set raw mode")?;

        let mut stdin = proc.stdin.take().unwrap();
        let resizer = proc.clone_resizer();

        let (engine_for_input, engine_for_output) = if predict_mode != PredictMode::Off {
            let engine = Arc::new(Mutex::new(PredictionEngine::new(predict_mode)));
            (Some(Arc::clone(&engine)), Some(engine))
        } else {
            (None, None)
        };

        let input_task = tokio::spawn(async move {
            input_loop(&mut terminal, &mut stdin, resizer, engine_for_input).await;
        });

        // Create output link with ConPTY sanitizer and optional prediction filter.
        let stdout_filter: StdoutFilter = if let Some(engine_out) = engine_for_output {
            Box::new(move |input, out| {
                let mut sanitized = Vec::with_capacity(input.len());
                TerminalSanitizer::CONPTY.filter(input, &mut sanitized);
                // Safety: lock poisoning indicates a panic in the other task;
                // propagating the panic here is the correct response.
                engine_out
                    .lock()
                    .expect("prediction engine lock poisoned")
                    .process_server_output(&sanitized, out);
            })
        } else {
            Box::new(|input, out| TerminalSanitizer::CONPTY.filter(input, out))
        };

        let link = RemoteProcessLink::from_remote_pipes_filtered(
            None,
            proc.stdout.take().unwrap(),
            proc.stderr.take().unwrap(),
            max_chunk_size,
            stdout_filter,
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

        // Reset SGR attributes and any blocked modes on the local terminal.
        let reset = TerminalSanitizer::CONPTY.reset_sequence();
        {
            use std::io::Write;
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            // Clear residual SGR state (underline, color) from predictions.
            let _ = out.write_all(b"\x1b[0m");
            if !reset.is_empty() {
                let _ = out.write_all(&reset);
            }
            let _ = out.flush();
        }
    }
}

/// Input handling loop: reads terminal events and forwards them to the remote process.
///
/// When `engine` is `Some`, keystrokes are fed to the prediction engine and
/// speculative local echo is written to stdout before the server round-trip
/// completes. Resize events are also forwarded to the engine so it can track
/// terminal width.
async fn input_loop(
    terminal: &mut impl Terminal,
    stdin: &mut RemoteStdin,
    resizer: RemoteProcessResizer,
    engine: Option<Arc<Mutex<PredictionEngine>>>,
) {
    while let Ok(input) = terminal.poll_input(Some(Duration::new(0, 0))) {
        match input {
            Some(InputEvent::Key(ev)) => {
                if let Ok(encoded) = ev.key.encode(
                    ev.modifiers,
                    KeyCodeEncodeModes {
                        encoding: KeyboardEncoding::Xterm,
                        application_cursor_keys: false,
                        newline_mode: false,
                        modify_other_keys: None,
                    },
                    /* is_down */ true,
                ) {
                    // Feed the encoded keystroke to the prediction engine.
                    if let Some(ref engine) = engine {
                        let mut guard = engine.lock().expect("prediction engine lock poisoned");
                        let action = guard.on_keystroke(&encoded);
                        let should_display = guard.should_display();
                        let should_underline = guard.should_underline();
                        drop(guard);

                        if should_display {
                            use std::io::Write;
                            let stdout = std::io::stdout();
                            let mut out = stdout.lock();
                            match action {
                                PredictionAction::DisplayChar(ch) => {
                                    if should_underline {
                                        let _ = out.write_all(b"\x1b[4m");
                                        let _ = write!(out, "{}", ch);
                                        let _ = out.write_all(b"\x1b[24m");
                                    } else {
                                        let _ = write!(out, "{}", ch);
                                    }
                                    let _ = out.flush();
                                }
                                PredictionAction::DisplayBackspace => {
                                    let _ = out.write_all(b"\x1b[D \x1b[D");
                                    let _ = out.flush();
                                }
                                _ => {}
                            }
                        }
                    }

                    if let Err(x) = stdin.write_str(encoded).await {
                        error!("Failed to write to stdin of remote process: {}", x);
                        break;
                    }
                }
            }
            Some(InputEvent::Resized { cols, rows }) => {
                if let Some(ref engine) = engine {
                    engine
                        .lock()
                        .expect("prediction engine lock poisoned")
                        .resize(cols);
                }
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

    #[test]
    fn filter_should_strip_focus_tracking_enable() {
        let input = b"\x1b[?1004h";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip ?1004h: {out:?}");
    }

    #[test]
    fn filter_should_strip_focus_tracking_disable() {
        let input = b"\x1b[?1004l";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip ?1004l: {out:?}");
    }

    #[test]
    fn filter_should_strip_win32_input_mode_enable() {
        let input = b"\x1b[?9001h";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip ?9001h: {out:?}");
    }

    #[test]
    fn filter_should_strip_win32_input_mode_disable() {
        let input = b"\x1b[?9001l";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip ?9001l: {out:?}");
    }

    #[test]
    fn filter_should_strip_xtwinops_resize() {
        let input = b"\x1b[8;24;80t";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip XTWINOPS: {out:?}");
    }

    #[test]
    fn filter_should_pass_through_normal_text() {
        let input = b"hello world";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    #[test]
    fn filter_should_pass_through_standard_csi_sequences() {
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
    fn filter_should_strip_blocked_from_mixed_content() {
        let mut input = Vec::new();
        input.extend_from_slice(b"hello");
        input.extend_from_slice(b"\x1b[?1004h");
        input.extend_from_slice(b"world");

        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(&input, &mut out);
        assert_eq!(out, b"helloworld");
    }

    #[test]
    fn filter_should_pass_through_empty_input() {
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(b"", &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn filter_should_strip_multiple_sequences() {
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
    fn filter_should_pass_through_incomplete_esc_at_end() {
        let input = b"text\x1b";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec());
    }

    #[test]
    fn filter_should_strip_normal_mouse_tracking() {
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
    fn filter_should_strip_button_event_mouse_tracking() {
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
    fn filter_should_strip_any_event_mouse_tracking() {
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
    fn filter_should_strip_utf8_mouse_mode() {
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
    fn filter_should_strip_sgr_extended_mouse_mode() {
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
    fn filter_should_strip_urxvt_mouse_mode() {
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
    fn filter_should_strip_mouse_modes_in_mixed_content() {
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

    #[test]
    fn scan_sequence_should_return_none_for_non_csi() {
        assert!(TerminalSanitizer::CONPTY.scan_sequence(b"hello").is_none());
        assert!(TerminalSanitizer::CONPTY.scan_sequence(b"\x1bO").is_none());
    }

    #[test]
    fn scan_sequence_should_return_none_for_unblocked_modes() {
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
    fn scan_sequence_should_return_length_for_1004h() {
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?1004h"),
            Some(8)
        );
    }

    #[test]
    fn scan_sequence_should_return_length_for_9001l() {
        assert_eq!(
            TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?9001l"),
            Some(8)
        );
    }

    #[test]
    fn scan_sequence_should_return_length_for_mouse_modes() {
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
    fn scan_sequence_should_return_length_for_xtwinops() {
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
    fn scan_sequence_should_return_none_for_incomplete_sequence() {
        assert!(TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[").is_none());
        assert!(TerminalSanitizer::CONPTY.scan_sequence(b"\x1b[?").is_none());
        assert!(
            TerminalSanitizer::CONPTY
                .scan_sequence(b"\x1b[?1004")
                .is_none()
        );
    }

    #[test]
    fn reset_sequence_should_disable_all_blocked_modes() {
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
    fn reset_sequence_should_use_l_suffix() {
        let reset = TerminalSanitizer::CONPTY.reset_sequence();
        let reset_str = String::from_utf8(reset).unwrap();

        // Should not contain any 'h' (enable) sequences
        assert!(
            !reset_str.contains('h'),
            "reset should only use 'l' (disable) suffix"
        );
    }

    #[test]
    fn filter_should_strip_osc_foreground_query_bel() {
        let input = b"\x1b]10;?\x07";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip OSC 10 query (BEL): {out:?}");
    }

    #[test]
    fn filter_should_strip_osc_foreground_query_st() {
        let input = b"\x1b]10;?\x1b\\";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip OSC 10 query (ST): {out:?}");
    }

    #[test]
    fn filter_should_strip_osc_background_query() {
        let input = b"\x1b]11;?\x07";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip OSC 11 query: {out:?}");
    }

    #[test]
    fn filter_should_strip_osc_cursor_color_query() {
        let input = b"\x1b]12;?\x07";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip OSC 12 query: {out:?}");
    }

    #[test]
    fn filter_should_strip_osc_palette_query() {
        let input = b"\x1b]4;5;?\x07";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip OSC 4 palette query: {out:?}");
    }

    #[test]
    fn filter_should_pass_through_osc_title_set() {
        let input = b"\x1b]0;My Title\x07";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec(), "OSC title set should pass through");
    }

    #[test]
    fn filter_should_pass_through_osc_clipboard() {
        let input = b"\x1b]52;c;data\x07";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec(), "OSC clipboard should pass through");
    }

    #[test]
    fn filter_should_pass_through_osc_color_set() {
        let input = b"\x1b]10;rgb:ff/ff/ff\x07";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec(), "OSC color set should pass through");
    }

    #[test]
    fn filter_should_strip_da1() {
        let input = b"\x1b[c";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip DA1: {out:?}");
    }

    #[test]
    fn filter_should_strip_da1_with_zero() {
        let input = b"\x1b[0c";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip DA1 with 0: {out:?}");
    }

    #[test]
    fn filter_should_strip_da2() {
        let input = b"\x1b[>c";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip DA2: {out:?}");
    }

    #[test]
    fn filter_should_strip_da2_with_zero() {
        let input = b"\x1b[>0c";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip DA2 with 0: {out:?}");
    }

    #[test]
    fn filter_should_strip_da3() {
        let input = b"\x1b[=c";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip DA3: {out:?}");
    }

    #[test]
    fn filter_should_strip_dsr_device_status() {
        let input = b"\x1b[5n";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip DSR device status: {out:?}");
    }

    #[test]
    fn filter_should_strip_dsr_cursor() {
        let input = b"\x1b[6n";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip DSR cursor position: {out:?}");
    }

    #[test]
    fn filter_should_strip_xtversion() {
        let input = b"\x1b[>q";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip XTVERSION: {out:?}");
    }

    #[test]
    fn filter_should_strip_xtversion_with_zero() {
        let input = b"\x1b[>0q";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip XTVERSION with 0: {out:?}");
    }

    #[test]
    fn filter_should_strip_decrqm() {
        let input = b"\x1b[?2026$p";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip DECRQM ?2026: {out:?}");
    }

    #[test]
    fn filter_should_strip_decrqm_other_modes() {
        for mode in ["2027", "2031", "2048"] {
            let input = format!("\x1b[?{mode}$p");
            let mut out = Vec::new();
            TerminalSanitizer::CONPTY.filter(input.as_bytes(), &mut out);
            assert!(out.is_empty(), "should strip DECRQM ?{mode}: {out:?}");
        }
    }

    #[test]
    fn filter_should_strip_queries_in_mixed_content() {
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b[1;32m"); // SGR green — pass through
        input.extend_from_slice(b"hello");
        input.extend_from_slice(b"\x1b[c"); // DA1 — strip
        input.extend_from_slice(b"\x1b]11;?\x07"); // OSC 11 query — strip
        input.extend_from_slice(b" world");
        input.extend_from_slice(b"\x1b[?2026$p"); // DECRQM — strip
        input.extend_from_slice(b"\x1b[6n"); // DSR — strip

        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(&input, &mut out);
        assert_eq!(out, b"\x1b[1;32mhello world");
    }

    #[test]
    fn filter_should_pass_through_incomplete_osc_at_end() {
        let input = b"text\x1b]10;";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec(), "incomplete OSC should pass through");
    }
}
