//! Framebuffer-based terminal renderer with predictive echo overlay.
//!
//! Owns a vt100 parser (read-only shadow screen) and a prediction overlay.
//! Server output bytes are sanitized and passed through to stdout directly.
//! Predicted characters are rendered via cursor escape sequences rather than
//! a full-screen diff renderer.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use log::trace;

use super::keyencode::encode_key;
use super::predict::{PredictMode, RttEstimator};
use super::terminal::TerminalSanitizer;

/// Maximum age before a pending prediction is discarded.
const PREDICTION_EXPIRY: Duration = Duration::from_secs(2);

/// Bulk paste detection: time window for counting input bytes.
const BULK_PASTE_WINDOW: Duration = Duration::from_millis(10);

/// Bulk paste detection: byte count threshold within the window.
const BULK_PASTE_THRESHOLD: usize = 100;

/// RTT threshold for adaptive mode activation.
const ADAPTIVE_RTT_THRESHOLD: Duration = Duration::from_millis(30);

/// RTT threshold above which predicted characters are underlined.
const UNDERLINE_RTT_THRESHOLD: Duration = Duration::from_millis(80);

/// A predicted character overlaid on the terminal screen.
struct PredictedCell {
    row: u16,
    col: u16,
    ch: char,
    epoch: u64,
    sent_at: Instant,
}

/// Manages prediction overlay state without byte-level parsing.
///
/// Predictions are placed at positions computed from the vt100 cursor
/// position plus an offset for pending predictions. Confirmation happens
/// by comparing the vt100 screen state after server bytes arrive.
struct PredictionOverlay {
    mode: PredictMode,
    rtt: RttEstimator,
    epoch_counter: u64,
    confirmed_epoch: u64,
    cells: Vec<PredictedCell>,
    last_input_time: Option<Instant>,
    input_byte_count: usize,
}

/// Direct byte passthrough terminal renderer with predictive echo.
///
/// Shared between input and output tasks via `Arc<Mutex<>>`. The input
/// side calls [`on_keystroke`](Self::on_keystroke) to add predictions and
/// get encoded bytes plus display bytes. The output side calls
/// [`process_server_output`](Self::process_server_output) to sanitize
/// bytes, update the shadow screen, and confirm predictions. Sanitized
/// bytes flow directly to stdout without an intermediate diff renderer.
pub struct TerminalFramebuffer {
    vt_parser: vt100::Parser,
    overlay: PredictionOverlay,
    sanitizer: TerminalSanitizer,
}

impl TerminalFramebuffer {
    /// Create a new framebuffer with the given terminal dimensions.
    ///
    /// Does NOT enter raw mode or alternate screen — the caller handles
    /// raw mode via crossterm.
    pub fn new(rows: u16, cols: u16, predict_mode: PredictMode) -> Self {
        let vt_parser = vt100::Parser::new(rows, cols, 0);

        Self {
            vt_parser,
            overlay: PredictionOverlay::new(predict_mode),
            sanitizer: TerminalSanitizer::CONPTY,
        }
    }

    /// Sanitize server output bytes, update the shadow screen, and confirm
    /// predictions. Returns the sanitized bytes for the caller to write
    /// directly to stdout.
    pub fn process_server_output(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut sanitized = Vec::with_capacity(bytes.len());
        self.sanitizer.filter(bytes, &mut sanitized);

        self.vt_parser.process(&sanitized);
        self.overlay.confirm_predictions(self.vt_parser.screen());

        sanitized
    }

    /// Record a user keystroke: add prediction, return encoded bytes to send
    /// to the server and display bytes to write to stdout for the prediction
    /// overlay.
    ///
    /// Returns `None` if the key is unrepresentable (modifier-only, media,
    /// etc.). The first element of the tuple is the encoded string to send
    /// to the remote process. The second element contains escape sequences
    /// to render the prediction overlay on the local terminal (empty if
    /// predictions are suppressed).
    pub fn on_keystroke(&mut self, event: &KeyEvent) -> Option<(String, Vec<u8>)> {
        let encoded = encode_key(event)?;
        self.overlay.on_input(&encoded, self.vt_parser.screen());

        let display_bytes = if self.overlay.should_display()
            && !self.in_alternate_screen()
            && !self.overlay.cells.is_empty()
        {
            self.build_prediction_display_bytes()
        } else {
            Vec::new()
        };

        Some((encoded, display_bytes))
    }

    /// Handle terminal resize.
    ///
    /// Updates the shadow screen size and clears pending predictions since
    /// their positions are no longer valid.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.vt_parser.screen_mut().set_size(rows, cols);
        self.overlay.cells.clear();
    }

    /// Returns `true` if the shadow screen is in alternate screen mode.
    ///
    /// Used to suppress predictions during full-screen applications like
    /// vim, less, etc.
    pub fn in_alternate_screen(&self) -> bool {
        self.vt_parser.screen().alternate_screen()
    }

    /// Restore terminal state on shutdown. Writes SGR reset and sanitizer
    /// reset sequences to stdout.
    pub fn shutdown(self) {
        let reset = self.sanitizer.reset_sequence();
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let _ = out.write_all(b"\x1b[0m");
        if !reset.is_empty() {
            let _ = out.write_all(&reset);
        }
        let _ = out.flush();
    }

    /// Build escape sequences to display all pending predicted characters.
    ///
    /// Uses DECSC/DECRC (save/restore cursor) to overlay predictions without
    /// disturbing the actual cursor position. If underline is active, wraps
    /// the predicted characters in SGR underline on/off sequences.
    fn build_prediction_display_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        let underline = self.overlay.should_underline();

        // Save cursor position (DECSC)
        buf.extend_from_slice(b"\x1b7");

        if underline {
            // SGR underline on
            buf.extend_from_slice(b"\x1b[4m");
        }

        for pred in &self.overlay.cells {
            // Move cursor to prediction position
            // CUP is 1-based: ESC[<row+1>;<col+1>H
            let row1 = pred.row + 1;
            let col1 = pred.col + 1;
            buf.extend_from_slice(format!("\x1b[{row1};{col1}H").as_bytes());

            let mut char_buf = [0u8; 4];
            let s = pred.ch.encode_utf8(&mut char_buf);
            buf.extend_from_slice(s.as_bytes());
        }

        if underline {
            // SGR underline off
            buf.extend_from_slice(b"\x1b[24m");
        }

        // Restore cursor position (DECRC)
        buf.extend_from_slice(b"\x1b8");

        buf
    }
}

impl PredictionOverlay {
    fn new(mode: PredictMode) -> Self {
        Self {
            mode,
            rtt: RttEstimator::new(),
            epoch_counter: 0,
            confirmed_epoch: 0,
            cells: Vec::new(),
            last_input_time: None,
            input_byte_count: 0,
        }
    }

    fn should_display(&self) -> bool {
        match self.mode {
            PredictMode::Off => false,
            PredictMode::On => true,
            PredictMode::Adaptive => self.rtt.srtt() >= ADAPTIVE_RTT_THRESHOLD,
        }
    }

    fn should_underline(&self) -> bool {
        self.should_display() && self.rtt.srtt() >= UNDERLINE_RTT_THRESHOLD
    }

    /// Process an encoded keystroke: detect bulk paste, handle epoch
    /// boundaries, and place predictions for printable characters.
    fn on_input(&mut self, encoded: &str, screen: &vt100::Screen) {
        if self.mode == PredictMode::Off {
            return;
        }

        // Bulk paste detection
        let now = Instant::now();
        if let Some(last) = self.last_input_time {
            if now.duration_since(last) < BULK_PASTE_WINDOW {
                self.input_byte_count += encoded.len();
                if self.input_byte_count >= BULK_PASTE_THRESHOLD {
                    self.cells.clear();
                    self.last_input_time = Some(now);
                    return;
                }
            } else {
                self.input_byte_count = encoded.len();
            }
        } else {
            self.input_byte_count = encoded.len();
        }
        self.last_input_time = Some(now);

        // Classify the input
        if encoded.len() == 1 {
            let b = encoded.as_bytes()[0];
            match b {
                // Epoch boundaries: Enter, Escape, Tab, and control chars
                b'\r' | b'\n' | 0x1b | b'\t' => {
                    self.new_epoch();
                    return;
                }
                // Backspace: remove last prediction
                0x7f | 0x08 => {
                    self.cells.pop();
                    return;
                }
                // Other control characters (Ctrl+A through Ctrl+Z minus already handled)
                0x01..=0x07 | 0x0b..=0x0c | 0x0e..=0x1a => {
                    self.new_epoch();
                    return;
                }
                // Printable ASCII
                0x20..=0x7e => {
                    self.add_prediction(b as char, screen);
                    return;
                }
                _ => {}
            }
        }

        // Multi-byte escape sequences (arrows, function keys) → epoch boundary
        if encoded.starts_with("\x1b[") || encoded.starts_with("\x1bO") {
            self.new_epoch();
            return;
        }

        // Multi-byte UTF-8 printable character
        if let Some(ch) = encoded.chars().next()
            && !ch.is_control()
        {
            self.add_prediction(ch, screen);
        }
    }

    fn new_epoch(&mut self) {
        self.epoch_counter += 1;
        self.cells.clear();
    }

    fn add_prediction(&mut self, ch: char, screen: &vt100::Screen) {
        // Tentative epoch: if the current epoch is ahead of confirmed by >1,
        // don't display (password prompt suppression).
        if self.epoch_counter > self.confirmed_epoch + 1 {
            trace!(
                "Tentative epoch {}: suppressing prediction '{}'",
                self.epoch_counter, ch
            );
            return;
        }

        let (base_row, base_col) = screen.cursor_position();
        let (_, term_width) = screen.size();
        let offset = self.cells.len() as u16;
        let total_col = base_col + offset;
        let (pred_row, pred_col) = if total_col >= term_width {
            (
                base_row.saturating_add(total_col / term_width),
                total_col % term_width,
            )
        } else {
            (base_row, total_col)
        };

        self.cells.push(PredictedCell {
            row: pred_row,
            col: pred_col,
            ch,
            epoch: self.epoch_counter,
            sent_at: Instant::now(),
        });
    }

    /// Confirm or discard predictions by comparing against the vt100 screen.
    fn confirm_predictions(&mut self, screen: &vt100::Screen) {
        let (cursor_row, cursor_col) = screen.cursor_position();

        // Destructure to satisfy the borrow checker: cells is mutably borrowed
        // by retain, while rtt and confirmed_epoch are borrowed by the closure.
        let Self {
            cells,
            rtt,
            confirmed_epoch,
            ..
        } = self;

        cells.retain(|pred| {
            // Expire old predictions
            if pred.sent_at.elapsed() > PREDICTION_EXPIRY {
                return false;
            }

            // Check if the screen now shows the predicted character
            if let Some(cell) = screen.cell(pred.row, pred.col)
                && cell.contents().starts_with(pred.ch)
            {
                rtt.update(pred.sent_at.elapsed());
                *confirmed_epoch = (*confirmed_epoch).max(pred.epoch);
                return false;
            }

            // Cursor has passed this position without matching → failed
            if pred.row < cursor_row || (pred.row == cursor_row && pred.col < cursor_col) {
                return false;
            }

            true
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay_on() -> PredictionOverlay {
        PredictionOverlay::new(PredictMode::On)
    }

    fn overlay_off() -> PredictionOverlay {
        PredictionOverlay::new(PredictMode::Off)
    }

    fn overlay_adaptive() -> PredictionOverlay {
        PredictionOverlay::new(PredictMode::Adaptive)
    }

    /// Create an overlay with epoch 0 confirmed so predictions display.
    fn overlay_on_confirmed() -> PredictionOverlay {
        let mut o = overlay_on();
        o.confirmed_epoch = 0;
        o
    }

    fn parser_80x24() -> vt100::Parser {
        vt100::Parser::new(24, 80, 0)
    }

    #[test]
    fn prediction_should_be_placed_at_cursor() {
        let mut o = overlay_on_confirmed();
        let parser = parser_80x24();
        o.on_input("a", parser.screen());
        assert_eq!(o.cells.len(), 1);
        assert_eq!(o.cells[0].ch, 'a');
        assert_eq!(o.cells[0].row, 0);
        assert_eq!(o.cells[0].col, 0);
    }

    #[test]
    fn predictions_should_offset_column_for_each_char() {
        let mut o = overlay_on_confirmed();
        let parser = parser_80x24();
        o.on_input("a", parser.screen());
        o.on_input("b", parser.screen());
        o.on_input("c", parser.screen());
        assert_eq!(o.cells.len(), 3);
        assert_eq!(o.cells[0].col, 0);
        assert_eq!(o.cells[1].col, 1);
        assert_eq!(o.cells[2].col, 2);
    }

    #[test]
    fn enter_should_create_epoch_boundary() {
        let mut o = overlay_on_confirmed();
        let parser = parser_80x24();
        o.on_input("a", parser.screen());
        assert_eq!(o.cells.len(), 1);
        o.on_input("\r", parser.screen());
        assert!(o.cells.is_empty());
        assert_eq!(o.epoch_counter, 1);
    }

    #[test]
    fn backspace_should_remove_last_prediction() {
        let mut o = overlay_on_confirmed();
        let parser = parser_80x24();
        o.on_input("a", parser.screen());
        o.on_input("b", parser.screen());
        assert_eq!(o.cells.len(), 2);
        o.on_input("\x7f", parser.screen());
        assert_eq!(o.cells.len(), 1);
        assert_eq!(o.cells[0].ch, 'a');
    }

    #[test]
    fn off_mode_should_not_add_predictions() {
        let mut o = overlay_off();
        let parser = parser_80x24();
        o.on_input("a", parser.screen());
        assert!(o.cells.is_empty());
    }

    #[test]
    fn confirmation_should_remove_matching_prediction() {
        let mut o = overlay_on_confirmed();
        let mut parser = parser_80x24();
        o.on_input("x", parser.screen());
        assert_eq!(o.cells.len(), 1);

        // Simulate server echoing back "x"
        parser.process(b"x");
        o.confirm_predictions(parser.screen());
        assert!(o.cells.is_empty());
    }

    #[test]
    fn confirmation_should_update_rtt() {
        let mut o = overlay_on_confirmed();
        let mut parser = parser_80x24();
        let initial_srtt = o.rtt.srtt();
        o.on_input("x", parser.screen());
        // Simulate small delay then echo
        parser.process(b"x");
        o.confirm_predictions(parser.screen());
        // RTT should have been updated (will be very small since no real delay)
        let _ = o.rtt.srtt(); // Just verify no panic
        // SRTT should change from the update
        assert_ne!(o.rtt.srtt(), initial_srtt);
    }

    #[test]
    fn cursor_past_prediction_should_remove_it() {
        let mut o = overlay_on_confirmed();
        let mut parser = parser_80x24();
        o.on_input("a", parser.screen());

        // Server sends something different that moves cursor past col 0
        parser.process(b"xy");
        o.confirm_predictions(parser.screen());
        assert!(o.cells.is_empty());
    }

    #[test]
    fn adaptive_should_not_display_with_low_rtt() {
        let mut o = overlay_adaptive();
        // Drive SRTT below the 30ms threshold
        for _ in 0..50 {
            o.rtt.update(Duration::from_millis(1));
        }
        assert!(!o.should_display());
    }

    #[test]
    fn adaptive_should_display_with_high_rtt() {
        let mut o = overlay_adaptive();
        // Drive SRTT above the 30ms threshold
        for _ in 0..50 {
            o.rtt.update(Duration::from_millis(100));
        }
        assert!(o.should_display());
    }

    #[test]
    fn should_underline_requires_higher_rtt() {
        let mut o = overlay_on();
        // With default 100ms SRTT, should_display is true, underline depends on threshold
        assert!(o.should_display());
        assert!(o.should_underline()); // 100ms > 80ms threshold

        // Lower the RTT below underline threshold but above display threshold
        for _ in 0..50 {
            o.rtt.update(Duration::from_millis(50));
        }
        assert!(o.should_display());
        assert!(!o.should_underline());
    }

    #[test]
    fn tentative_epoch_should_suppress_predictions() {
        let mut o = overlay_on_confirmed();
        let parser = parser_80x24();

        // epoch 0 is confirmed. Create epoch 1 (e.g., after Enter)
        o.on_input("\r", parser.screen());
        assert_eq!(o.epoch_counter, 1);

        // Now type in epoch 1 — this is fine (epoch 1 = confirmed_epoch + 1)
        o.on_input("p", parser.screen());
        // epoch 1 hasn't been confirmed yet but is only 1 ahead → allowed
        assert_eq!(o.cells.len(), 1);

        // Create epoch 2 without confirming epoch 1
        o.on_input("\r", parser.screen());
        assert_eq!(o.epoch_counter, 2);

        // Now type in epoch 2 — this is tentative (2 > 0 + 1)
        o.on_input("s", parser.screen());
        assert!(o.cells.is_empty(), "tentative epoch should suppress");
    }

    #[test]
    fn bulk_paste_should_clear_predictions() {
        let mut o = overlay_on_confirmed();
        let parser = parser_80x24();

        // Simulate rapid input exceeding threshold
        let big_input: String = "a".repeat(BULK_PASTE_THRESHOLD);
        for ch in big_input.chars() {
            o.on_input(&ch.to_string(), parser.screen());
        }
        assert!(
            o.cells.is_empty(),
            "bulk paste should have cleared predictions"
        );
    }

    #[test]
    fn arrow_key_should_create_epoch_boundary() {
        let mut o = overlay_on_confirmed();
        let parser = parser_80x24();
        o.on_input("a", parser.screen());
        assert_eq!(o.cells.len(), 1);
        o.on_input("\x1b[A", parser.screen()); // Up arrow
        assert!(o.cells.is_empty());
        assert_eq!(o.epoch_counter, 1);
    }

    #[test]
    fn alternate_screen_should_suppress_predictions() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        assert!(!fb.in_alternate_screen());

        // Enter alternate screen
        fb.process_server_output(b"\x1b[?1049h");
        assert!(fb.in_alternate_screen());

        // Keystroke while in alternate screen should produce empty display bytes
        let ev = KeyEvent {
            code: crossterm::event::KeyCode::Char('a'),
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        let (encoded, display_bytes) = fb.on_keystroke(&ev).unwrap();
        assert_eq!(encoded, "a");
        assert!(
            display_bytes.is_empty(),
            "predictions should be suppressed in alternate screen, got: {:?}",
            display_bytes
        );

        // Exit alternate screen
        fb.process_server_output(b"\x1b[?1049l");
        assert!(!fb.in_alternate_screen());
    }

    #[test]
    fn process_server_output_should_return_sanitized_bytes() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::Off);

        // Input with a blocked DEC private mode (?1004h = focus tracking)
        let input = b"hello\x1b[?1004hworld";
        let sanitized = fb.process_server_output(input);
        assert_eq!(
            sanitized, b"helloworld",
            "blocked ?1004h sequence should be stripped"
        );
    }

    #[test]
    fn on_keystroke_should_return_prediction_display_bytes() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        let ev = KeyEvent {
            code: crossterm::event::KeyCode::Char('a'),
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        let (encoded, display_bytes) = fb.on_keystroke(&ev).unwrap();

        assert_eq!(encoded, "a");

        // Expected: DECSC + SGR underline on + CUP(1,1) + 'a' + SGR underline off + DECRC
        // Cursor starts at (0,0), CUP is 1-based so row=1, col=1.
        // Underline is active because default SRTT (100ms) exceeds the 80ms threshold.
        let expected = b"\x1b7\x1b[4m\x1b[1;1Ha\x1b[24m\x1b8";
        assert_eq!(
            display_bytes,
            expected,
            "display bytes should be DECSC + underline + CUP + char + no-underline + DECRC, got: {:?}",
            String::from_utf8_lossy(&display_bytes)
        );
    }

    #[test]
    fn on_keystroke_should_return_none_for_modifier_only_key() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        let ev = KeyEvent {
            code: crossterm::event::KeyCode::Modifier(crossterm::event::ModifierKeyCode::LeftShift),
            modifiers: crossterm::event::KeyModifiers::SHIFT,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        assert!(
            fb.on_keystroke(&ev).is_none(),
            "modifier-only key should return None"
        );
    }

    #[test]
    fn resize_should_clear_pending_predictions() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // Add a prediction
        let ev = KeyEvent {
            code: crossterm::event::KeyCode::Char('x'),
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        let result = fb.on_keystroke(&ev);
        assert!(result.is_some(), "keystroke should produce output");

        // Resize should clear predictions
        fb.resize(120, 40);
        assert!(
            fb.overlay.cells.is_empty(),
            "resize should clear pending predictions"
        );
    }
}
