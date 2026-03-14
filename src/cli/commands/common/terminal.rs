use std::io;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use crossterm::event::{self, Event, KeyEventKind};
use distant_core::protocol::PtySize;
use distant_core::{RemoteProcess, RemoteProcessResizer, RemoteStdin};
use log::*;
use tokio::task::JoinHandle;

use super::framebuffer::TerminalFramebuffer;
use super::predict::PredictMode;
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
    /// XTVERSION, Kitty keyboard query, DECRQSS, and OSC queries ending with `?`).
    strip_queries: bool,
}

/// Classified escape sequence extracted by `parse_escape`.
enum EscapeSeq<'a> {
    /// CSI sequence: `ESC [` private_marker? params intermediates final_byte.
    Csi {
        private_marker: Option<u8>,
        params: &'a [u8],
        intermediates: &'a [u8],
        final_byte: u8,
    },
    /// OSC sequence: `ESC ]` content (BEL | ST).
    Osc { content: &'a [u8] },
    /// DCS sequence: `ESC P` content (BEL | ST).
    Dcs { content: &'a [u8] },
    /// SS3 sequence: `ESC O` byte.
    Ss3,
    /// Two-character escape: `ESC` byte.
    TwoChar,
}

/// Action to take for a parsed escape sequence.
enum SeqAction {
    Strip,
    PassThrough,
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
    /// Scans `input` for escape sequences (CSI, OSC, DCS, SS3, two-char)
    /// and classifies each one. Blocked sequences are silently dropped;
    /// everything else is passed through unchanged into `out`.
    pub fn filter(&self, input: &[u8], out: &mut Vec<u8>) {
        let mut i = 0;
        while i < input.len() {
            if input[i] == 0x1b
                && let Some((seq, len)) = Self::parse_escape(&input[i..])
            {
                match self.classify(&seq) {
                    SeqAction::Strip => {
                        trace!("Filtered terminal sequence: {:02x?}", &input[i..i + len]);
                    }
                    SeqAction::PassThrough => {
                        out.extend_from_slice(&input[i..i + len]);
                    }
                }
                i += len;
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

    /// Classify a parsed escape sequence as Strip or PassThrough.
    ///
    /// All filtering decisions are centralized in this single match.
    fn classify(&self, seq: &EscapeSeq) -> SeqAction {
        match seq {
            // ── Terminal queries (strip_queries) ──

            // DA1: ESC[c or ESC[0c
            EscapeSeq::Csi {
                private_marker: None,
                params,
                final_byte: b'c',
                ..
            } if self.strip_queries && (params.is_empty() || *params == [b'0']) => SeqAction::Strip,

            // DA2: ESC[>c or ESC[>0c
            EscapeSeq::Csi {
                private_marker: Some(b'>'),
                params,
                final_byte: b'c',
                ..
            } if self.strip_queries && (params.is_empty() || *params == [b'0']) => SeqAction::Strip,

            // DA3: ESC[=c
            EscapeSeq::Csi {
                private_marker: Some(b'='),
                final_byte: b'c',
                ..
            } if self.strip_queries => SeqAction::Strip,

            // DSR: ESC[5n or ESC[6n
            EscapeSeq::Csi {
                private_marker: None,
                params,
                final_byte: b'n',
                ..
            } if self.strip_queries && (*params == [b'5'] || *params == [b'6']) => SeqAction::Strip,

            // XTVERSION: ESC[>q or ESC[>0q
            EscapeSeq::Csi {
                private_marker: Some(b'>'),
                params,
                final_byte: b'q',
                ..
            } if self.strip_queries && (params.is_empty() || *params == [b'0']) => SeqAction::Strip,

            // DECRQM: ESC[?<digits>$p
            EscapeSeq::Csi {
                private_marker: Some(b'?'),
                intermediates,
                final_byte: b'p',
                ..
            } if self.strip_queries && *intermediates == [b'$'] => SeqAction::Strip,

            // Kitty keyboard query: ESC[?u (no params)
            EscapeSeq::Csi {
                private_marker: Some(b'?'),
                params,
                final_byte: b'u',
                ..
            } if self.strip_queries && params.is_empty() => SeqAction::Strip,

            // OSC queries: content ends with '?'
            EscapeSeq::Osc { content } if self.strip_queries && content.last() == Some(&b'?') => {
                SeqAction::Strip
            }

            // DECRQSS: ESC P $ q ... ST
            EscapeSeq::Dcs { content } if self.strip_queries && content.starts_with(b"$q") => {
                SeqAction::Strip
            }

            // ── Blocked DEC private modes ──

            // ESC[?<mode>h or ESC[?<mode>l
            EscapeSeq::Csi {
                private_marker: Some(b'?'),
                params,
                final_byte,
                ..
            } if (*final_byte == b'h' || *final_byte == b'l')
                && Self::is_blocked_mode(params, self.blocked_modes) =>
            {
                SeqAction::Strip
            }

            // ── Blocked resize ──

            // XTWINOPS: ESC[8;<rows>;<cols>t
            EscapeSeq::Csi {
                private_marker: None,
                params,
                final_byte: b't',
                ..
            } if self.strip_xtwinops && Self::is_xtwinops_resize(params) => SeqAction::Strip,

            // ── Everything else passes through ──
            _ => SeqAction::PassThrough,
        }
    }

    /// Parse an escape sequence starting at `buf[0] == ESC`.
    ///
    /// Returns the classified sequence and its total byte length, or `None`
    /// if the sequence is incomplete or unrecognized.
    fn parse_escape(buf: &[u8]) -> Option<(EscapeSeq<'_>, usize)> {
        if buf.len() < 2 || buf[0] != 0x1b {
            return None;
        }

        match buf[1] {
            b'[' => Self::parse_csi(buf),
            b']' => Self::parse_osc(buf),
            b'P' => Self::parse_dcs(buf),
            b'O' => {
                if buf.len() < 3 {
                    return None;
                }
                Some((EscapeSeq::Ss3, 3))
            }
            // Two-char escapes (DECSC, DECRC, etc.)
            b if b.is_ascii_graphic() || b == b' ' => Some((EscapeSeq::TwoChar, 2)),
            _ => None,
        }
    }

    /// Parse a CSI sequence: `ESC [` private_marker? params intermediates final.
    fn parse_csi(buf: &[u8]) -> Option<(EscapeSeq<'_>, usize)> {
        if buf.len() < 3 {
            return None;
        }

        let mut j = 2;

        // Optional private marker: ?, >, =
        let private_marker = if j < buf.len() && matches!(buf[j], b'?' | b'>' | b'=') {
            j += 1;
            Some(buf[j - 1])
        } else {
            None
        };

        // Parameters: digits, ;, :
        let params_start = j;
        while j < buf.len() && (buf[j].is_ascii_digit() || buf[j] == b';' || buf[j] == b':') {
            j += 1;
        }
        let params = &buf[params_start..j];

        // Intermediates: 0x20..=0x2F ($, #, space, etc.)
        let intermediates_start = j;
        while j < buf.len() && (0x20..=0x2F).contains(&buf[j]) {
            j += 1;
        }
        let intermediates = &buf[intermediates_start..j];

        // Final byte: 0x40..=0x7E
        if j >= buf.len() || !(0x40..=0x7E).contains(&buf[j]) {
            return None;
        }
        let final_byte = buf[j];

        Some((
            EscapeSeq::Csi {
                private_marker,
                params,
                intermediates,
                final_byte,
            },
            j + 1,
        ))
    }

    /// Parse an OSC sequence: `ESC ]` content (BEL | `ESC \`).
    fn parse_osc(buf: &[u8]) -> Option<(EscapeSeq<'_>, usize)> {
        if buf.len() < 2 || buf[0] != 0x1b || buf[1] != b']' {
            return None;
        }

        let mut j = 2;
        while j < buf.len() {
            match buf[j] {
                0x07 => {
                    return Some((
                        EscapeSeq::Osc {
                            content: &buf[2..j],
                        },
                        j + 1,
                    ));
                }
                0x1b => {
                    if j + 1 < buf.len() && buf[j + 1] == b'\\' {
                        return Some((
                            EscapeSeq::Osc {
                                content: &buf[2..j],
                            },
                            j + 2,
                        ));
                    }
                    return None;
                }
                _ => j += 1,
            }
        }

        // Incomplete — no terminator found
        None
    }

    /// Parse a DCS sequence: `ESC P` content (BEL | `ESC \`).
    fn parse_dcs(buf: &[u8]) -> Option<(EscapeSeq<'_>, usize)> {
        if buf.len() < 2 || buf[0] != 0x1b || buf[1] != b'P' {
            return None;
        }

        let mut j = 2;
        while j < buf.len() {
            match buf[j] {
                0x07 => {
                    return Some((
                        EscapeSeq::Dcs {
                            content: &buf[2..j],
                        },
                        j + 1,
                    ));
                }
                0x1b => {
                    if j + 1 < buf.len() && buf[j + 1] == b'\\' {
                        return Some((
                            EscapeSeq::Dcs {
                                content: &buf[2..j],
                            },
                            j + 2,
                        ));
                    }
                    return None;
                }
                _ => j += 1,
            }
        }

        // Incomplete — no terminator found
        None
    }

    /// Check if CSI params represent a blocked DEC private mode number.
    fn is_blocked_mode(params: &[u8], blocked_modes: &[u32]) -> bool {
        if let Ok(s) = std::str::from_utf8(params)
            && let Ok(mode) = s.parse::<u32>()
        {
            return blocked_modes.contains(&mode);
        }
        false
    }

    /// Check if CSI params represent an XTWINOPS resize: `8;<rows>;<cols>`.
    fn is_xtwinops_resize(params: &[u8]) -> bool {
        let s = match std::str::from_utf8(params) {
            Ok(s) => s,
            Err(_) => return false,
        };

        let mut parts = s.split(';');
        if parts.next() != Some("8") {
            return false;
        }
        let rows = match parts.next().and_then(|p| p.parse::<u32>().ok()) {
            Some(r) if r > 0 => r,
            _ => return false,
        };
        let cols = match parts.next().and_then(|p| p.parse::<u32>().ok()) {
            Some(c) if c > 0 => c,
            _ => return false,
        };
        let _ = (rows, cols);
        parts.next().is_none()
    }
}

/// Manages the local terminal for an interactive remote shell session.
///
/// Handles crossterm raw mode setup, input forwarding (keys to remote stdin,
/// resize events to remote PTY), framebuffer-based rendering with predictive
/// echo, and terminal cleanup on shutdown.
///
/// All three `Shell::spawn()` call sites (distant shell, distant spawn --pty,
/// distant ssh) go through this struct, ensuring consistent terminal handling.
pub struct TerminalSession {
    _input_task: JoinHandle<()>,
    link: RemoteProcessLink,
    framebuffer: Option<Arc<Mutex<TerminalFramebuffer>>>,
}

impl TerminalSession {
    /// Start a terminal session for the given remote process.
    ///
    /// Takes ownership of the process's stdin/stdout/stderr pipes.
    /// Sets the local terminal to raw mode via crossterm, creates a
    /// framebuffer renderer, spawns an input handler task (forwarding key
    /// events and resize events), and creates a filtered output link.
    ///
    /// # Errors
    ///
    /// Returns an error if raw mode cannot be enabled or the framebuffer
    /// cannot be created.
    pub fn start(
        proc: &mut RemoteProcess,
        max_chunk_size: usize,
        predict_mode: PredictMode,
    ) -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode().context("Failed to enable raw mode")?;

        let (cols, rows) = crossterm::terminal::size().context("Failed to get terminal size")?;

        let framebuffer = Arc::new(Mutex::new(TerminalFramebuffer::new(
            rows,
            cols,
            predict_mode,
        )));

        let mut stdin = proc.stdin.take().unwrap();
        let resizer = proc.clone_resizer();
        let fb_for_input = Arc::clone(&framebuffer);

        let input_task = tokio::spawn(async move {
            input_loop(&mut stdin, resizer, fb_for_input).await;
        });

        // Stdout filter: processes server output through the framebuffer
        // (sanitization + prediction erase/re-display) and writes directly
        // to stdout while holding the framebuffer lock.
        let fb_for_output = Arc::clone(&framebuffer);
        let stdout_filter: StdoutFilter = Box::new(move |input| {
            let mut fb = fb_for_output.lock().expect("framebuffer lock poisoned");
            let output = fb.render_server_output(input);
            if !output.is_empty() {
                let stdout = io::stdout();
                let mut out = stdout.lock();
                let _ = out.write_all(&output);
                let _ = out.flush();
            }
        });

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
            framebuffer: Some(framebuffer),
        })
    }

    /// Shut down the session: drain output, then reset terminal modes.
    ///
    /// Disables crossterm raw mode and writes reset sequences to stdout to
    /// disable any DEC private modes that may have been enabled by the
    /// remote host (mouse tracking, etc.).
    pub async fn shutdown(self) {
        self.link.shutdown().await;

        let _ = crossterm::terminal::disable_raw_mode();

        if let Some(fb) = self.framebuffer
            && let Ok(fb) = Arc::try_unwrap(fb)
        {
            let fb = fb.into_inner().expect("framebuffer lock poisoned");
            fb.shutdown();
        }
    }
}

/// Input handling loop: reads crossterm events and forwards them to the
/// remote process.
///
/// Key events are fed to the framebuffer (which handles prediction overlay
/// and rendering) and the encoded bytes are sent to remote stdin. Resize
/// events update both the framebuffer and the remote PTY size.
async fn input_loop(
    stdin: &mut RemoteStdin,
    resizer: RemoteProcessResizer,
    framebuffer: Arc<Mutex<TerminalFramebuffer>>,
) {
    loop {
        match event::poll(Duration::ZERO) {
            Ok(true) => match event::read() {
                Ok(Event::Key(ev)) => {
                    if ev.kind == KeyEventKind::Release {
                        continue;
                    }

                    let encoded = {
                        let mut fb = framebuffer.lock().expect("framebuffer lock poisoned");
                        let result = fb.on_keystroke(&ev);
                        if let Some((_, ref display_bytes)) = result
                            && !display_bytes.is_empty()
                        {
                            let stdout = io::stdout();
                            let mut out = stdout.lock();
                            let _ = out.write_all(display_bytes);
                            let _ = out.flush();
                        }
                        result.map(|(e, _)| e)
                    };

                    if let Some(encoded) = encoded
                        && let Err(x) = stdin.write_str(encoded).await
                    {
                        error!("Failed to write to stdin of remote process: {}", x);
                        break;
                    }
                }
                Ok(Event::Resize(cols, rows)) => {
                    framebuffer
                        .lock()
                        .expect("framebuffer lock poisoned")
                        .resize(cols, rows);
                    if let Err(x) = resizer
                        .resize(PtySize::from_rows_and_cols(rows, cols))
                        .await
                    {
                        error!("Failed to resize remote process: {}", x);
                        break;
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            },
            Ok(false) => {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(_) => break,
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

    #[test]
    fn filter_should_strip_kitty_keyboard_query() {
        let input = b"\x1b[?u";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip kitty keyboard query: {out:?}");
    }

    #[test]
    fn filter_should_strip_decrqss_sgr_query() {
        let input = b"\x1bP$qm\x1b\\";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(out.is_empty(), "should strip DECRQSS SGR query: {out:?}");
    }

    #[test]
    fn filter_should_strip_decrqss_in_mixed() {
        let mut input = Vec::new();
        input.extend_from_slice(b"hello");
        input.extend_from_slice(b"\x1bP$qm\x1b\\");
        input.extend_from_slice(b"world");

        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(&input, &mut out);
        assert_eq!(out, b"helloworld");
    }

    #[test]
    fn filter_should_pass_through_non_query_dcs() {
        // DECDLD (font loading) — not a query, should pass through
        let input = b"\x1bPfoo\x1b\\";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec(), "non-query DCS should pass through");
    }

    #[test]
    fn filter_should_pass_through_ss3_sequences() {
        let input = b"\x1bOA";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec(), "SS3 sequences should pass through");
    }

    #[test]
    fn filter_should_pass_through_two_char_escapes() {
        // DECSC (ESC 7) and DECRC (ESC 8)
        let input = b"\x1b7\x1b8";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert_eq!(out, input.to_vec(), "two-char escapes should pass through");
    }

    #[test]
    fn filter_should_strip_kitty_query_in_mixed() {
        let mut input = Vec::new();
        input.extend_from_slice(b"\x1b[1;31m"); // SGR — pass through
        input.extend_from_slice(b"text");
        input.extend_from_slice(b"\x1b[?u"); // Kitty query — strip
        input.extend_from_slice(b"\x1bP$qm\x1b\\"); // DECRQSS — strip
        input.extend_from_slice(b" more");

        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(&input, &mut out);
        assert_eq!(out, b"\x1b[1;31mtext more");
    }

    #[test]
    fn filter_should_strip_decrqss_with_bel_terminator() {
        let input = b"\x1bP$qm\x07";
        let mut out = Vec::new();
        TerminalSanitizer::CONPTY.filter(input, &mut out);
        assert!(
            out.is_empty(),
            "should strip DECRQSS with BEL terminator: {out:?}"
        );
    }
}
