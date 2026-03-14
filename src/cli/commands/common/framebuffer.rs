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

/// A predicted backspace erasure.
struct BackspacePrediction {
    row: u16,
    col: u16,
    /// Original character at this position (from shadow screen).
    original_char: String,
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
    backspace_predictions: Vec<BackspacePrediction>,
    last_input_time: Option<Instant>,
    input_byte_count: usize,
    /// Cursor position at the start of user input within the current epoch.
    /// Backspace predictions cannot go below this column on the same row,
    /// preventing the user from visually deleting the shell prompt.
    input_floor: Option<(u16, u16)>,
}

/// Direct byte passthrough terminal renderer with predictive echo.
///
/// Shared between input and output tasks via `Arc<Mutex<>>`. The input
/// side calls [`on_keystroke`](Self::on_keystroke) to add predictions and
/// get encoded bytes plus display bytes. The output side calls
/// [`render_server_output`](Self::render_server_output) to erase displayed
/// predictions, sanitize and apply server bytes, and re-display remaining
/// predictions — all returned as a single atomic byte sequence for stdout.
pub struct TerminalFramebuffer {
    vt_parser: vt100::Parser,
    overlay: PredictionOverlay,
    sanitizer: TerminalSanitizer,
    displayed_count: usize,
    backspace_displayed: usize,
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
            displayed_count: 0,
            backspace_displayed: 0,
        }
    }

    /// Process server output with prediction lifecycle management.
    /// Returns bytes to write atomically to stdout: erase old predictions,
    /// server output, and re-display of remaining predictions.
    pub fn render_server_output(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();

        // Erase displayed predictions before server output
        if self.displayed_count > 0 || self.backspace_displayed > 0 {
            self.append_erase(&mut output);
        }

        // Sanitize, parse, confirm (delegates to existing method)
        let sanitized = self.process_server_output(bytes);
        output.extend_from_slice(&sanitized);

        // Re-display remaining predictions after server output
        if self.overlay.should_display()
            && !self.in_alternate_screen()
            && (!self.overlay.cells.is_empty() || !self.overlay.backspace_predictions.is_empty())
        {
            self.append_predictions(&mut output);
        }

        output
    }

    /// Sanitize server output bytes, update the shadow screen, and confirm
    /// predictions. Returns the sanitized bytes. Used internally by
    /// `render_server_output` and in tests.
    fn process_server_output(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut sanitized = Vec::with_capacity(bytes.len());
        self.sanitizer.filter(bytes, &mut sanitized);

        self.vt_parser.process(&sanitized);
        self.overlay.confirm_predictions(self.vt_parser.screen());
        self.overlay
            .confirm_backspace_predictions(self.vt_parser.screen());

        // Epoch recovery: if no predictions are pending, synchronize
        // confirmed_epoch so future predictions aren't permanently suppressed.
        if self.overlay.cells.is_empty()
            && self.overlay.backspace_predictions.is_empty()
            && self.overlay.epoch_counter > self.overlay.confirmed_epoch + 1
        {
            self.overlay.confirmed_epoch = self.overlay.epoch_counter.saturating_sub(1);
        }

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
        let app_cursor = self.vt_parser.screen().application_cursor();
        let encoded = encode_key(event, app_cursor)?;

        let epoch_before = self.overlay.epoch_counter;
        self.overlay.on_input(&encoded, self.vt_parser.screen());
        let is_epoch_boundary = self.overlay.epoch_counter != epoch_before;

        let mut display_bytes = Vec::new();

        let has_new = self.overlay.should_display()
            && !self.in_alternate_screen()
            && (!self.overlay.cells.is_empty() || !self.overlay.backspace_predictions.is_empty());

        // Erase old predictions ONLY when:
        // - Displaying new predictions (erase + redisplay), OR
        // - A non-epoch-boundary keystroke with no new predictions
        //   (e.g., backspace removes last prediction)
        //
        // For epoch boundaries (Enter, Escape, etc.) with no new predictions,
        // leave predictions visible. render_server_output will erase them
        // atomically with server output, preventing visible flash.
        let anything_displayed = self.displayed_count > 0 || self.backspace_displayed > 0;
        if anything_displayed && (has_new || !is_epoch_boundary) {
            self.append_erase(&mut display_bytes);
        }

        // Display new predictions
        if has_new {
            self.append_predictions(&mut display_bytes);
        }

        Some((encoded, display_bytes))
    }

    /// Handle terminal resize.
    ///
    /// Updates the shadow screen size and clears pending predictions since
    /// their positions are no longer valid.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.vt_parser.screen_mut().set_size(rows, cols);
        self.overlay.cells.clear();
        self.overlay.backspace_predictions.clear();
        self.overlay.input_floor = None;
        self.displayed_count = 0;
        self.backspace_displayed = 0;
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

    /// Erase currently-displayed prediction chars from the terminal.
    /// Restores cursor to server position (DECRC), restores original
    /// characters for backspace predictions, writes spaces over forward
    /// predictions, then restores cursor again.
    fn append_erase(&mut self, buf: &mut Vec<u8>) {
        // DECRC — go to saved server cursor position
        buf.extend_from_slice(b"\x1b8");
        // Reset SGR (clear any underline from predictions)
        buf.extend_from_slice(b"\x1b[m");

        let bs = self.backspace_displayed;
        let fwd = self.displayed_count;

        if bs > 0 {
            // Clamp to available predictions — new_epoch() may have cleared
            // the vec while backspace_displayed is still nonzero.
            let available = self.overlay.backspace_predictions.len().min(bs);

            // write! to Vec<u8> is infallible
            write!(buf, "\x1b[{}D", bs).unwrap();
            // Restore original characters where still available
            for bp in &self.overlay.backspace_predictions[..available] {
                buf.extend_from_slice(bp.original_char.as_bytes());
            }
            // Fill remaining positions with spaces if predictions were cleared
            if available < bs {
                buf.resize(buf.len() + (bs - available), b' ');
            }
            // Cursor is now at server cursor position
        }

        // Erase forward predictions past server cursor
        if fwd > bs {
            buf.resize(buf.len() + (fwd - bs), b' ');
        }

        // DECRC — return to server cursor position
        buf.extend_from_slice(b"\x1b8");
        self.displayed_count = 0;
        self.backspace_displayed = 0;
    }

    /// Write prediction display: DECSC + [underline] + chars + [underline off].
    /// Does NOT emit DECRC — cursor stays at predicted position (fixes cursor lag).
    fn append_predictions(&mut self, buf: &mut Vec<u8>) {
        // DECSC — save server cursor position
        buf.extend_from_slice(b"\x1b7");

        let underline = self.overlay.should_underline();
        if underline {
            buf.extend_from_slice(b"\x1b[4m");
        }

        let bs_count = self.overlay.backspace_predictions.len();
        let fwd_count = self.overlay.cells.len();

        // write! to Vec<u8> is infallible (Vec's Write impl never errors)
        if bs_count > 0 && fwd_count == 0 {
            // Backspace only — erase characters behind cursor
            write!(buf, "\x1b[{}D", bs_count).unwrap();
            buf.resize(buf.len() + bs_count, b' ');
            write!(buf, "\x1b[{}D", bs_count).unwrap();
        } else if bs_count > 0 && fwd_count > 0 {
            // Combined — backspace + forward
            write!(buf, "\x1b[{}D", bs_count).unwrap();
            for pred in &self.overlay.cells {
                let mut char_buf = [0u8; 4];
                let s = pred.ch.encode_utf8(&mut char_buf);
                buf.extend_from_slice(s.as_bytes());
            }
            if fwd_count < bs_count {
                let remaining = bs_count - fwd_count;
                buf.resize(buf.len() + remaining, b' ');
                write!(buf, "\x1b[{}D", remaining).unwrap();
            }
        } else {
            // Forward only (existing behavior)
            for pred in &self.overlay.cells {
                let mut char_buf = [0u8; 4];
                let s = pred.ch.encode_utf8(&mut char_buf);
                buf.extend_from_slice(s.as_bytes());
            }
        }

        if underline {
            buf.extend_from_slice(b"\x1b[24m");
        }

        // NO DECRC — leave cursor at predicted position
        self.displayed_count = fwd_count;
        self.backspace_displayed = bs_count;
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
            backspace_predictions: Vec::new(),
            last_input_time: None,
            input_byte_count: 0,
            input_floor: None,
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
                    self.backspace_predictions.clear();
                    self.input_floor = None;
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
                // Backspace: remove last prediction cell, or predict erasure
                0x7f | 0x08 => {
                    if !self.cells.is_empty() {
                        self.cells.pop();
                    } else {
                        // Tentative epoch check (password suppression)
                        if self.epoch_counter > self.confirmed_epoch + 1 {
                            return;
                        }

                        let (cursor_row, cursor_col) = screen.cursor_position();
                        if self.input_floor.is_none() {
                            self.input_floor = Some((cursor_row, cursor_col));
                        }
                        let predicted_col =
                            cursor_col as i32 - self.backspace_predictions.len() as i32;

                        let floor_col = self
                            .input_floor
                            .filter(|(fr, _)| *fr == cursor_row)
                            .map(|(_, fc)| fc as i32)
                            .unwrap_or(cursor_col as i32);

                        if predicted_col > floor_col {
                            let target_col = (predicted_col - 1) as u16;
                            if let Some(cell) = screen.cell(cursor_row, target_col) {
                                let content = cell.contents();
                                if !content.is_empty() && content.len() <= 4 {
                                    self.backspace_predictions.push(BackspacePrediction {
                                        row: cursor_row,
                                        col: target_col,
                                        original_char: content.to_string(),
                                        epoch: self.epoch_counter,
                                        sent_at: Instant::now(),
                                    });
                                }
                            }
                        }
                    }
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
        if self.cells.is_empty() && self.backspace_predictions.is_empty() {
            self.confirmed_epoch = self.epoch_counter.saturating_sub(1);
        }
        self.cells.clear();
        self.backspace_predictions.clear();
        self.input_floor = None;
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
        if self.input_floor.is_none() {
            self.input_floor = Some((base_row, base_col));
        }
        let (_, term_width) = screen.size();
        let backspace_offset = self.backspace_predictions.len() as u16;
        let effective_col = base_col.saturating_sub(backspace_offset);
        let offset = self.cells.len() as u16;
        let total_col = effective_col + offset;
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

    /// Confirm or discard backspace predictions by checking cursor position.
    fn confirm_backspace_predictions(&mut self, screen: &vt100::Screen) {
        let (cursor_row, cursor_col) = screen.cursor_position();
        let Self {
            backspace_predictions,
            rtt,
            confirmed_epoch,
            ..
        } = self;

        backspace_predictions.retain(|bp| {
            if bp.sent_at.elapsed() > PREDICTION_EXPIRY {
                return false;
            }
            // Server cursor moved back to/past this position → confirmed
            if cursor_row < bp.row || (cursor_row == bp.row && cursor_col <= bp.col) {
                rtt.update(bp.sent_at.elapsed());
                *confirmed_epoch = (*confirmed_epoch).max(bp.epoch);
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

        // Expected: DECSC + SGR underline on + 'a' + SGR underline off (NO DECRC).
        // No CUP — chars are written sequentially from the current cursor position.
        // Underline is active because default SRTT (100ms) exceeds the 80ms threshold.
        // Cursor stays at prediction end (no DECRC) to fix cursor lag.
        let expected = b"\x1b7\x1b[4ma\x1b[24m";
        assert_eq!(
            display_bytes,
            expected,
            "display bytes should be DECSC + underline + char + no-underline (no DECRC), got: {:?}",
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
        let (encoded, _display_bytes) = fb
            .on_keystroke(&ev)
            .expect("keystroke should produce output");
        assert_eq!(encoded, "x");

        // Resize should clear predictions
        fb.resize(120, 40);
        assert!(
            fb.overlay.cells.is_empty(),
            "resize should clear pending predictions"
        );
    }

    #[test]
    fn display_bytes_should_not_contain_cup_sequences() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // Move cursor to a non-origin position via server output
        fb.process_server_output(b"\x1b[10;20H$ ");

        let ev = KeyEvent {
            code: crossterm::event::KeyCode::Char('x'),
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        let (_, display_bytes) = fb.on_keystroke(&ev).unwrap();

        let display_str = String::from_utf8_lossy(&display_bytes);
        // CUP pattern: ESC[ <digits> ; <digits> H
        assert!(
            !display_str.contains(";") || !display_str.contains('H'),
            "display bytes should not contain CUP sequences, got: {display_str:?}"
        );
    }

    #[test]
    fn epoch_should_recover_when_cells_empty_on_new_epoch() {
        let mut o = overlay_on_confirmed();
        let parser = parser_80x24();

        // Create multiple epoch boundaries without any predictions between them.
        // Each new_epoch should catch up confirmed_epoch since cells are empty.
        o.on_input("\r", parser.screen()); // epoch 1, cells were empty
        o.on_input("\r", parser.screen()); // epoch 2, cells were empty
        o.on_input("\r", parser.screen()); // epoch 3, cells were empty

        // confirmed_epoch should have caught up
        assert!(
            o.epoch_counter <= o.confirmed_epoch + 1,
            "confirmed_epoch should catch up: epoch={}, confirmed={}",
            o.epoch_counter,
            o.confirmed_epoch
        );

        // Predictions should now be allowed
        o.on_input("a", parser.screen());
        assert_eq!(
            o.cells.len(),
            1,
            "predictions should be allowed after epoch recovery"
        );
    }

    #[test]
    fn epoch_should_recover_after_server_output_clears_cells() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // Type a character (adds prediction in epoch 0)
        let ev = KeyEvent {
            code: crossterm::event::KeyCode::Char('a'),
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        fb.on_keystroke(&ev);

        // Enter creates epoch 1 (cells had 'a' so no catch-up)
        let enter = KeyEvent {
            code: crossterm::event::KeyCode::Enter,
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        fb.on_keystroke(&enter);

        // Another Enter creates epoch 2 (cells were empty -> catch-up)
        fb.on_keystroke(&enter);

        // Server output echoes back and moves cursor past predictions
        fb.process_server_output(b"a\r\n$ ");

        // Epoch recovery in process_server_output should catch up
        assert!(
            fb.overlay.epoch_counter <= fb.overlay.confirmed_epoch + 1,
            "confirmed_epoch should catch up after server output: epoch={}, confirmed={}",
            fb.overlay.epoch_counter,
            fb.overlay.confirmed_epoch
        );

        // New predictions should be allowed
        fb.on_keystroke(&ev);
        assert!(
            !fb.overlay.cells.is_empty(),
            "predictions should resume after epoch recovery via server output"
        );
    }

    #[test]
    fn display_bytes_should_write_all_chars_sequentially() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        fb.process_server_output(b"$ ");

        // Type 'a', 'b', 'c' — each keystroke erases old predictions and re-displays all
        let mut last_display_bytes = Vec::new();
        for ch in ['a', 'b', 'c'] {
            let (_, display_bytes) = fb.on_keystroke(&key_char(ch)).unwrap();
            last_display_bytes = display_bytes;
        }

        let display_str = String::from_utf8_lossy(&last_display_bytes);

        // Should contain 'abc' in the re-displayed predictions
        assert!(
            display_str.contains("abc"),
            "chars should appear sequentially, got: {display_str:?}"
        );

        // Should NOT end with DECRC — cursor stays at prediction end
        assert!(
            !last_display_bytes.ends_with(b"\x1b8"),
            "should NOT end with DECRC (cursor stays at prediction end)"
        );
    }

    #[test]
    fn prediction_should_wrap_at_end_of_line() {
        let mut o = overlay_on_confirmed();
        // Use a narrow terminal (10 cols) with cursor near end of line
        let mut parser = vt100::Parser::new(24, 10, 0);
        // Move cursor to row 5, col 8 (2 cols from edge)
        parser.process(b"\x1b[6;9H");

        o.on_input("a", parser.screen()); // col 8
        o.on_input("b", parser.screen()); // col 9 (last col)
        o.on_input("c", parser.screen()); // should wrap to row 6, col 0

        assert_eq!(o.cells.len(), 3);
        assert_eq!((o.cells[0].row, o.cells[0].col), (5, 8));
        assert_eq!((o.cells[1].row, o.cells[1].col), (5, 9));
        assert_eq!(
            (o.cells[2].row, o.cells[2].col),
            (6, 0),
            "third prediction should wrap to next row"
        );
    }

    fn key_char(ch: char) -> KeyEvent {
        KeyEvent {
            code: crossterm::event::KeyCode::Char(ch),
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn key_enter() -> KeyEvent {
        KeyEvent {
            code: crossterm::event::KeyCode::Enter,
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    fn key_ctrl_c() -> KeyEvent {
        KeyEvent {
            code: crossterm::event::KeyCode::Char('c'),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    /// Feed server output through framebuffer and to the display verifier.
    /// Uses `render_server_output` so erase + re-display bytes flow through.
    fn feed_server(fb: &mut TerminalFramebuffer, display: &mut vt100::Parser, bytes: &[u8]) {
        let output = fb.render_server_output(bytes);
        display.process(&output);
    }

    /// Send a keystroke and feed display bytes to the verifier.
    /// Returns the encoded string (for the "server" side).
    fn feed_keystroke(
        fb: &mut TerminalFramebuffer,
        display: &mut vt100::Parser,
        ev: &KeyEvent,
    ) -> Option<String> {
        let (encoded, display_bytes) = fb.on_keystroke(ev)?;
        if !display_bytes.is_empty() {
            display.process(&display_bytes);
        }
        Some(encoded)
    }

    /// Read a row from the display verifier as a trimmed string.
    fn display_row(display: &vt100::Parser, row: u16) -> String {
        let screen = display.screen();
        let (_, cols) = screen.size();
        let mut text = String::new();
        for col in 0..cols {
            if let Some(cell) = screen.cell(row, col) {
                let c = cell.contents();
                text.push_str(if c.is_empty() { " " } else { c });
            }
        }
        text.trim_end().to_string()
    }

    #[test]
    fn predictions_should_appear_at_cursor_not_at_origin() {
        // Real terminal has 30 lines of prior output.
        let mut display = vt100::Parser::new(40, 80, 0);
        for i in 0..30 {
            display.process(format!("previous line {i}\r\n").as_bytes());
        }
        // Display cursor is now at row 30.

        let mut fb = TerminalFramebuffer::new(40, 80, PredictMode::On);

        // Server sends prompt — both shadow screen and display get it.
        feed_server(&mut fb, &mut display, b"$ ");

        // User types 'l'.
        feed_keystroke(&mut fb, &mut display, &key_char('l'));

        // Row 0 must still say "previous line 0" — NOT overwritten by CUP.
        let row0 = display_row(&display, 0);
        assert!(
            row0.starts_with("previous line 0"),
            "row 0 should be unchanged, got: {row0:?}"
        );

        // Row 30 should show "$ " + predicted 'l'.
        let row30 = display_row(&display, 30);
        assert!(
            row30.starts_with("$ l"),
            "prediction should appear at cursor row, got: {row30:?}"
        );
    }

    #[test]
    fn passwd_interrupt_should_not_block_future_predictions() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        let mut display = vt100::Parser::new(24, 80, 0);

        // Server sends initial prompt.
        feed_server(&mut fb, &mut display, b"$ ");

        // User types "passwd\r" -> server echoes + sends Password prompt.
        for ch in "passwd".chars() {
            feed_keystroke(&mut fb, &mut display, &key_char(ch));
        }
        feed_keystroke(&mut fb, &mut display, &key_enter());
        feed_server(&mut fb, &mut display, b"passwd\r\nPassword: ");

        // User types password (no server echo).
        for ch in "secret".chars() {
            feed_keystroke(&mut fb, &mut display, &key_char(ch));
        }
        feed_keystroke(&mut fb, &mut display, &key_enter());

        // Server responds + new prompt.
        feed_server(&mut fb, &mut display, b"\r\nOK\r\n$ ");

        // Ctrl-C -> server sends new prompt.
        feed_keystroke(&mut fb, &mut display, &key_ctrl_c());
        feed_server(&mut fb, &mut display, b"^C\r\n$ ");

        // User types 'l' — predictions MUST resume.
        let (encoded, display_bytes) = fb.on_keystroke(&key_char('l')).unwrap();
        assert_eq!(encoded, "l");
        assert!(
            !display_bytes.is_empty(),
            "predictions should resume after epoch recovery"
        );
        assert!(
            display_bytes.contains(&b'l'),
            "display bytes should contain predicted 'l'"
        );

        // Verify 'l' appears in the display at the prompt row.
        display.process(&display_bytes);
        // Find the last row containing "$ " — that's our prompt row.
        let prompt_row = (0..24u16)
            .rev()
            .find(|&r| display_row(&display, r).contains("$ "))
            .expect("should find a prompt row");
        let row_text = display_row(&display, prompt_row);
        assert!(
            row_text.contains("$ l"),
            "prompt row should show predicted 'l', got: {row_text:?}"
        );
    }

    #[test]
    fn enter_should_not_erase_predictions_immediately() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        fb.process_server_output(b"$ ");

        // Type "abc" — predictions are displayed
        for ch in ['a', 'b', 'c'] {
            fb.on_keystroke(&key_char(ch));
        }
        assert_eq!(fb.displayed_count, 3);

        // Press Enter — epoch boundary, but erase is deferred
        let (_, display_bytes) = fb.on_keystroke(&key_enter()).unwrap();

        // Should NOT contain spaces (no immediate erase)
        assert!(
            !display_bytes.contains(&b' '),
            "display bytes should NOT contain spaces — erase is deferred"
        );
        // displayed_count stays at 3 — will be erased by render_server_output
        assert_eq!(
            fb.displayed_count, 3,
            "displayed_count should remain 3 (erase deferred)"
        );
    }

    #[test]
    fn erase_should_precede_server_output() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        fb.process_server_output(b"$ ");

        // Type "ls" — predictions displayed
        fb.on_keystroke(&key_char('l'));
        fb.on_keystroke(&key_char('s'));
        assert_eq!(fb.displayed_count, 2);

        // Server echoes "ls" — render_server_output should erase first
        let output = fb.render_server_output(b"ls");

        // Output should start with erase (DECRC), then contain "ls"
        assert!(
            output.starts_with(b"\x1b8"),
            "output should start with DECRC for erase"
        );
        // After erase, the sanitized server bytes should be present
        assert!(
            output.windows(2).any(|w| w == b"ls"),
            "output should contain server echo 'ls'"
        );
    }

    #[test]
    fn cursor_should_be_at_prediction_end() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        let mut display = vt100::Parser::new(24, 80, 0);

        feed_server(&mut fb, &mut display, b"$ ");
        feed_keystroke(&mut fb, &mut display, &key_char('a'));
        feed_keystroke(&mut fb, &mut display, &key_char('b'));
        feed_keystroke(&mut fb, &mut display, &key_char('c'));

        // Display verifier cursor should be at the end of "$ abc" = col 5
        let (_, cursor_col) = display.screen().cursor_position();
        assert_eq!(
            cursor_col, 5,
            "cursor should be at end of predictions (col 5), got col {cursor_col}"
        );
    }

    #[test]
    fn server_output_should_not_interleave_with_predictions() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        let mut display = vt100::Parser::new(24, 80, 0);

        // Server sends prompt
        feed_server(&mut fb, &mut display, b"$ ");

        // User types "ls"
        feed_keystroke(&mut fb, &mut display, &key_char('l'));
        feed_keystroke(&mut fb, &mut display, &key_char('s'));

        // Display should show "$ ls" at this point
        let row0 = display_row(&display, 0);
        assert!(
            row0.starts_with("$ ls"),
            "before echo, should show predicted 'ls', got: {row0:?}"
        );

        // Server echoes "ls" — render_server_output handles erase + output + re-display
        feed_server(&mut fb, &mut display, b"ls");

        // After echo, display should still show "$ ls" cleanly
        let row0 = display_row(&display, 0);
        assert!(
            row0.starts_with("$ ls"),
            "after echo, should show confirmed 'ls', got: {row0:?}"
        );
    }

    #[test]
    fn render_server_output_should_redisplay_remaining_predictions() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // Server sends prompt
        fb.process_server_output(b"$ ");

        // User types "abc"
        fb.on_keystroke(&key_char('a'));
        fb.on_keystroke(&key_char('b'));
        fb.on_keystroke(&key_char('c'));
        assert_eq!(fb.displayed_count, 3);

        // Server echoes only "a" — 'b' and 'c' still pending
        let output = fb.render_server_output(b"a");

        // Should have re-displayed remaining predictions ('b', 'c')
        assert_eq!(
            fb.displayed_count, 2,
            "should have 2 remaining predictions displayed"
        );

        // Output should contain the predicted chars
        let output_str = String::from_utf8_lossy(&output);
        assert!(
            output_str.contains("bc"),
            "output should contain remaining predictions 'bc', got: {output_str:?}"
        );
    }

    #[test]
    fn resize_should_reset_displayed_count() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        fb.process_server_output(b"$ ");

        fb.on_keystroke(&key_char('x'));
        assert_eq!(fb.displayed_count, 1);

        fb.resize(120, 40);
        assert_eq!(fb.displayed_count, 0, "resize should reset displayed_count");
    }

    #[test]
    fn render_server_output_should_erase_stale_predictions() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        fb.process_server_output(b"$ ");

        // Type "abc" — predictions displayed
        for ch in ['a', 'b', 'c'] {
            fb.on_keystroke(&key_char(ch));
        }
        assert_eq!(fb.displayed_count, 3);

        // Press Enter — erase is deferred, predictions stay visible
        fb.on_keystroke(&key_enter());
        assert_eq!(
            fb.displayed_count, 3,
            "displayed_count should remain 3 after Enter"
        );

        // Server echoes — render_server_output should erase stale predictions
        let output = fb.render_server_output(b"abc\r\n$ ");

        // Output should start with DECRC (erase)
        assert!(
            output.starts_with(b"\x1b8"),
            "output should start with DECRC for erase"
        );
        assert_eq!(
            fb.displayed_count, 0,
            "displayed_count should be 0 after server output"
        );
    }

    #[test]
    fn typing_after_enter_should_erase_and_redisplay() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        fb.process_server_output(b"$ ");

        // Type "abc" — predictions displayed
        for ch in ['a', 'b', 'c'] {
            fb.on_keystroke(&key_char(ch));
        }
        assert_eq!(fb.displayed_count, 3);

        // Press Enter — erase deferred
        fb.on_keystroke(&key_enter());
        assert_eq!(fb.displayed_count, 3);

        // Type 'l' before server responds — should erase old and display new
        let (_, display_bytes) = fb.on_keystroke(&key_char('l')).unwrap();

        // Should contain spaces (erasing old 3-char prediction)
        assert!(
            display_bytes.contains(&b' '),
            "display bytes should contain spaces for erasing old predictions"
        );
        // After erase + new prediction, displayed_count should be 1
        assert_eq!(
            fb.displayed_count, 1,
            "displayed_count should be 1 for new prediction"
        );
    }

    fn key_backspace() -> KeyEvent {
        KeyEvent {
            code: crossterm::event::KeyCode::Backspace,
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn backspace_should_predict_erasure_from_shadow_screen() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // User types "hello" (sets input floor at col 0)
        for ch in "hello".chars() {
            fb.on_keystroke(&key_char(ch));
        }
        // Server confirms "hello" — cursor at col 5
        fb.process_server_output(b"hello");
        assert!(fb.overlay.cells.is_empty());

        // Press backspace — should predict erasure of 'o' at col 4
        let (encoded, display_bytes) = fb.on_keystroke(&key_backspace()).unwrap();
        assert_eq!(encoded, "\x7f");

        assert_eq!(fb.overlay.backspace_predictions.len(), 1);
        assert_eq!(fb.overlay.backspace_predictions[0].original_char, "o");
        assert_eq!(fb.overlay.backspace_predictions[0].col, 4);
        assert_eq!(fb.overlay.backspace_predictions[0].row, 0);

        // Display bytes should contain CUB (cursor back) sequence
        assert!(
            !display_bytes.is_empty(),
            "display bytes should be non-empty for backspace prediction"
        );
        let display_str = String::from_utf8_lossy(&display_bytes);
        assert!(
            display_str.contains("\x1b[1D"),
            "display bytes should contain CUB sequence, got: {display_str:?}"
        );
    }

    #[test]
    fn backspace_should_not_predict_at_column_zero() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // No server output — cursor at col 0
        let (encoded, display_bytes) = fb.on_keystroke(&key_backspace()).unwrap();
        assert_eq!(encoded, "\x7f");

        assert!(
            fb.overlay.backspace_predictions.is_empty(),
            "no backspace prediction should be added at col 0"
        );
        assert!(
            display_bytes.is_empty(),
            "display bytes should be empty when no prediction is made"
        );
    }

    #[test]
    fn backspace_prediction_should_restore_on_server_echo() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // User types "hello" (sets floor at col 0), server confirms
        for ch in "hello".chars() {
            fb.on_keystroke(&key_char(ch));
        }
        fb.process_server_output(b"hello");

        // Press backspace — predicts erasure of 'o' at col 4
        fb.on_keystroke(&key_backspace());
        assert_eq!(fb.backspace_displayed, 1);
        assert_eq!(fb.overlay.backspace_predictions.len(), 1);
        assert_eq!(fb.overlay.backspace_predictions[0].original_char, "o");

        // Server echoes backspace (BS + space + BS erases the character)
        let output = fb.render_server_output(b"\x08 \x08");

        // After server echo, the backspace prediction should be confirmed
        // (server cursor moved back to col 4, which is <= bp.col 4)
        assert_eq!(
            fb.backspace_displayed, 0,
            "backspace_displayed should be 0 after server echo"
        );

        // Output should contain the erase sequence (DECRC) since predictions were displayed
        assert!(
            output.starts_with(b"\x1b8"),
            "output should start with DECRC for erasing displayed predictions"
        );
    }

    #[test]
    fn multiple_backspace_should_predict_multiple_erasures() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // User types "abc" (sets floor at col 0), server confirms
        for ch in "abc".chars() {
            fb.on_keystroke(&key_char(ch));
        }
        fb.process_server_output(b"abc");

        // Press backspace 3 times
        fb.on_keystroke(&key_backspace());
        fb.on_keystroke(&key_backspace());
        fb.on_keystroke(&key_backspace());

        assert_eq!(fb.overlay.backspace_predictions.len(), 3);

        // Original chars should be in order of backspace press
        assert_eq!(fb.overlay.backspace_predictions[0].original_char, "c");
        assert_eq!(fb.overlay.backspace_predictions[1].original_char, "b");
        assert_eq!(fb.overlay.backspace_predictions[2].original_char, "a");

        // Columns should go backwards
        assert_eq!(fb.overlay.backspace_predictions[0].col, 2);
        assert_eq!(fb.overlay.backspace_predictions[1].col, 1);
        assert_eq!(fb.overlay.backspace_predictions[2].col, 0);
    }

    #[test]
    fn typing_after_backspace_should_combine() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // User types "hello" (sets floor at col 0), server confirms
        for ch in "hello".chars() {
            fb.on_keystroke(&key_char(ch));
        }
        fb.process_server_output(b"hello");

        // Press backspace twice — erases 'o' at col 4 and 'l' at col 3
        fb.on_keystroke(&key_backspace());
        fb.on_keystroke(&key_backspace());
        assert_eq!(fb.overlay.backspace_predictions.len(), 2);

        // Type 'x' and 'y'
        fb.on_keystroke(&key_char('x'));
        fb.on_keystroke(&key_char('y'));

        // Backspace predictions remain
        assert_eq!(fb.overlay.backspace_predictions.len(), 2);
        // Forward predictions placed after backspace offset
        assert_eq!(fb.overlay.cells.len(), 2);
        assert_eq!(fb.overlay.cells[0].col, 3);
        assert_eq!(fb.overlay.cells[1].col, 4);
    }

    #[test]
    fn backspace_prediction_should_respect_tentative_epoch() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        fb.process_server_output(b"$ ");

        // Type "passwd" + Enter (creates epoch 1; cells non-empty -> no catch-up)
        for ch in "passwd".chars() {
            fb.on_keystroke(&key_char(ch));
        }
        fb.on_keystroke(&key_enter());

        // Type "x" in epoch 1 (allowed: epoch 1 <= confirmed 0 + 1)
        fb.on_keystroke(&key_char('x'));

        // Another Enter without server confirmation (creates epoch 2;
        // cells non-empty -> no catch-up)
        fb.on_keystroke(&key_enter());

        // Now epoch_counter=2, confirmed_epoch=0 => tentative (2 > 0+1)
        assert!(
            fb.overlay.epoch_counter > fb.overlay.confirmed_epoch + 1,
            "should be in tentative epoch: epoch={}, confirmed={}",
            fb.overlay.epoch_counter,
            fb.overlay.confirmed_epoch
        );

        // Press backspace — should NOT add backspace prediction in tentative epoch
        fb.on_keystroke(&key_backspace());
        assert!(
            fb.overlay.backspace_predictions.is_empty(),
            "backspace prediction should be suppressed in tentative epoch"
        );
    }

    #[test]
    fn backspace_then_enter_then_server_should_not_panic() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // User types "hello" (sets floor at col 0), server confirms
        for ch in "hello".chars() {
            fb.on_keystroke(&key_char(ch));
        }
        fb.process_server_output(b"hello");

        // Press backspace — backspace_displayed becomes 1
        fb.on_keystroke(&key_backspace());
        assert_eq!(fb.backspace_displayed, 1);

        // Press Enter — new_epoch clears backspace_predictions, but backspace_displayed stays
        fb.on_keystroke(&key_enter());
        assert!(
            fb.overlay.backspace_predictions.is_empty(),
            "new_epoch should clear backspace_predictions"
        );

        // Server output — should NOT panic even though backspace_predictions
        // is empty while backspace_displayed is nonzero
        let output = fb.render_server_output(b"hello\r\n$ ");
        assert_eq!(
            fb.backspace_displayed, 0,
            "backspace_displayed should be reset after render_server_output"
        );

        // Output should start with DECRC (erase) since things were displayed
        assert!(
            output.starts_with(b"\x1b8"),
            "output should start with DECRC for erase"
        );
    }

    #[test]
    fn backspace_should_not_delete_prompt() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // Server sends "$ " — cursor at col 2
        fb.process_server_output(b"$ ");

        // User types "hello" (sets input floor at col 2)
        for ch in "hello".chars() {
            fb.on_keystroke(&key_char(ch));
        }
        // Server confirms "hello" — cursor at col 7
        fb.process_server_output(b"hello");

        // Press backspace 5 times to delete "hello"
        for _ in 0..5 {
            fb.on_keystroke(&key_backspace());
        }
        assert_eq!(
            fb.overlay.backspace_predictions.len(),
            5,
            "should predict 5 backspaces for 'hello'"
        );

        // 6th backspace should NOT create a prediction (prompt boundary)
        fb.on_keystroke(&key_backspace());
        assert_eq!(
            fb.overlay.backspace_predictions.len(),
            5,
            "6th backspace should be blocked by input floor (prompt boundary)"
        );
    }

    #[test]
    fn backspace_at_prompt_without_typing_should_not_predict() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // Server sends "$ " — cursor at col 2
        fb.process_server_output(b"$ ");

        // Press backspace without typing anything first
        fb.on_keystroke(&key_backspace());
        assert!(
            fb.overlay.backspace_predictions.is_empty(),
            "backspace at prompt without prior typing should not predict"
        );
    }

    #[test]
    fn backspace_should_respect_floor_after_server_confirms() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // Server sends "$ "
        fb.process_server_output(b"$ ");

        // User types "hello" — sets input_floor to (0, 2)
        for ch in "hello".chars() {
            fb.on_keystroke(&key_char(ch));
        }

        // Server confirms "hello" — cursor at col 7, cells cleared
        fb.process_server_output(b"hello");
        assert!(fb.overlay.cells.is_empty(), "cells should be confirmed");

        // Now backspace 5 times (deletes "hello")
        for _ in 0..5 {
            fb.on_keystroke(&key_backspace());
        }
        assert_eq!(fb.overlay.backspace_predictions.len(), 5);

        // 6th backspace should be blocked by floor at col 2
        fb.on_keystroke(&key_backspace());
        assert_eq!(
            fb.overlay.backspace_predictions.len(),
            5,
            "floor should persist after server confirmation, blocking prompt deletion"
        );
    }

    #[test]
    fn backspace_floor_should_reset_on_enter() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);

        // Server sends "$ "
        fb.process_server_output(b"$ ");

        // User types "ls" — sets floor at col 2
        fb.on_keystroke(&key_char('l'));
        fb.on_keystroke(&key_char('s'));

        // Enter clears floor
        fb.on_keystroke(&key_enter());
        assert!(
            fb.overlay.input_floor.is_none(),
            "input_floor should be cleared by new_epoch"
        );

        // Server sends new prompt at different position
        fb.process_server_output(b"ls\r\n$ ");

        // User types "x" — sets new floor
        fb.on_keystroke(&key_char('x'));
        assert!(
            fb.overlay.input_floor.is_some(),
            "new floor should be set for new prompt"
        );
    }

    #[test]
    fn backspace_display_should_show_erasure_on_screen() {
        let mut fb = TerminalFramebuffer::new(24, 80, PredictMode::On);
        let mut display = vt100::Parser::new(24, 80, 0);

        // Feed server "$ " to both
        feed_server(&mut fb, &mut display, b"$ ");

        // User types "hello" (sets floor at col 2)
        for ch in "hello".chars() {
            feed_keystroke(&mut fb, &mut display, &key_char(ch));
        }
        // Server confirms "hello"
        feed_server(&mut fb, &mut display, b"hello");

        let row0 = display_row(&display, 0);
        assert_eq!(row0, "$ hello");

        // Press backspace via feed_keystroke
        feed_keystroke(&mut fb, &mut display, &key_backspace());

        // Display should show "$ hell" (the 'o' is erased)
        let row0 = display_row(&display, 0);
        assert_eq!(
            row0, "$ hell",
            "backspace prediction should erase 'o', got: {row0:?}"
        );

        // Feed server backspace echo
        feed_server(&mut fb, &mut display, b"\x08 \x08");

        // Display should still show "$ hell" (no double-delete)
        let row0 = display_row(&display, 0);
        assert_eq!(
            row0, "$ hell",
            "server echo should not cause double-delete, got: {row0:?}"
        );
    }
}
