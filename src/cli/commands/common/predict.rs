//! Mosh-style predictive echo engine.
//!
//! Provides local echo prediction for interactive remote shell sessions,
//! reducing perceived latency by displaying keystrokes immediately while
//! the round trip to the server completes. Predictions are confirmed or
//! rolled back as server output arrives.
//!
//! Key types:
//! - [`PredictionEngine`] — the main engine, designed for `Arc<Mutex<>>` sharing
//! - [`PredictMode`] — controls when prediction is active (off / adaptive / on)
//! - [`PredictionAction`] — returned from [`PredictionEngine::on_keystroke`]

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use log::trace;

/// Maximum age before a pending prediction is marked as failed.
const PREDICTION_EXPIRY: Duration = Duration::from_secs(2);

/// Bulk paste detection threshold: bytes accumulated within this window
/// trigger a reset to avoid displaying speculative characters during paste.
const BULK_PASTE_WINDOW: Duration = Duration::from_millis(10);

/// Bulk paste detection threshold: byte count within [`BULK_PASTE_WINDOW`].
const BULK_PASTE_THRESHOLD: usize = 100;

/// Controls whether the prediction engine displays speculative local echo.
///
/// In `Adaptive` mode (the default), predictions are shown only when the
/// measured round-trip time is high enough that the user would perceive
/// latency. `On` forces predictions unconditionally, while `Off` disables
/// them entirely.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum PredictMode {
    /// Never predict — all output comes from the server.
    Off,

    /// Predict based on measured RTT. Predictions activate when SRTT
    /// exceeds 30 ms.
    #[default]
    Adaptive,

    /// Always predict — every printable keystroke is echoed locally.
    On,
}

/// Jacobson/Karels smoothed RTT estimator.
///
/// Tracks a smoothed RTT (`srtt`) and RTT variance (`rttvar`) using the
/// classic TCP algorithm. Used by [`PredictionEngine`] to decide whether
/// adaptive prediction should be active.
pub struct RttEstimator {
    /// Smoothed round-trip time.
    srtt: Duration,
    /// RTT variance estimate.
    rttvar: Duration,
}

impl RttEstimator {
    /// Creates a new estimator with initial SRTT of 100 ms and variance of 50 ms.
    pub fn new() -> Self {
        Self {
            srtt: Duration::from_millis(100),
            rttvar: Duration::from_millis(50),
        }
    }

    /// Updates the estimator with a new RTT sample.
    ///
    /// Uses Jacobson/Karels algorithm with alpha = 1/8 and beta = 1/4.
    /// All arithmetic is saturating to avoid overflow panics on extreme values.
    pub fn update(&mut self, sample: Duration) {
        // rttvar = (1 - beta) * rttvar + beta * |srtt - sample|
        //        = 3/4 * rttvar + 1/4 * |srtt - sample|
        let diff = if sample > self.srtt {
            sample.saturating_sub(self.srtt)
        } else {
            self.srtt.saturating_sub(sample)
        };
        let three_quarters_var = self.rttvar.saturating_mul(3) / 4;
        let quarter_diff = diff / 4;
        self.rttvar = three_quarters_var.saturating_add(quarter_diff);

        // srtt = (1 - alpha) * srtt + alpha * sample
        //      = 7/8 * srtt + 1/8 * sample
        let seven_eighths_srtt = self.srtt.saturating_mul(7) / 8;
        let eighth_sample = sample / 8;
        self.srtt = seven_eighths_srtt.saturating_add(eighth_sample);
    }

    /// Returns the current smoothed RTT estimate.
    pub fn srtt(&self) -> Duration {
        self.srtt
    }
}

impl Default for RttEstimator {
    fn default() -> Self {
        Self::new()
    }
}

/// Lifecycle state of a single predicted character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PredictionState {
    /// Waiting for server confirmation.
    Pending,
    /// Server output matched the prediction.
    Confirmed,
    /// Server output did not match, or the prediction expired.
    Failed,
}

/// A single predicted character at a specific cursor position.
pub struct Prediction {
    /// The predicted character.
    pub ch: char,
    /// Cursor column where the character is expected to appear.
    pub col: usize,
    /// When the originating keystroke was sent.
    pub sent_at: Instant,
    /// Current state of this prediction.
    pub state: PredictionState,
    /// Epoch this prediction belongs to.
    #[allow(dead_code)]
    pub epoch: u64,
}

/// A group of predictions that share a contiguous, predictable input sequence.
///
/// An epoch boundary is created whenever the user types something unpredictable
/// (Enter, Escape, control characters, arrow keys, etc.).
pub struct Epoch {
    /// Monotonically increasing epoch identifier.
    pub id: u64,
    /// Predictions within this epoch, in keystroke order.
    pub predictions: VecDeque<Prediction>,
}

/// Action returned by [`PredictionEngine::on_keystroke`] indicating what the
/// caller should display locally.
#[derive(Debug, PartialEq, Eq)]
pub enum PredictionAction {
    /// Display this character at the cursor position as speculative echo.
    DisplayChar(char),
    /// Erase the previous character (backspace prediction).
    DisplayBackspace,
    /// An epoch boundary was crossed — unpredictable input was sent.
    NewEpoch,
    /// No prediction (e.g., bulk paste detected, or prediction disabled).
    None,
}

/// Mosh-style predictive echo engine.
///
/// Designed to be shared between input and output tasks via `Arc<Mutex<>>`.
/// The input side calls [`on_keystroke`](Self::on_keystroke) to record
/// predictions and receive display actions. The output side calls
/// [`process_server_output`](Self::process_server_output) to confirm or
/// roll back predictions as server bytes arrive.
pub struct PredictionEngine {
    /// Current prediction mode.
    mode: PredictMode,
    /// Confirmed cursor column (from server output).
    cursor_col: usize,
    /// Terminal width for line wrapping.
    term_width: usize,
    /// Monotonically increasing epoch counter.
    epoch_counter: u64,
    /// Highest epoch ID where at least one prediction has been confirmed.
    confirmed_epoch: u64,
    /// Active prediction epochs.
    epochs: VecDeque<Epoch>,
    /// RTT estimator for adaptive mode decisions.
    rtt: RttEstimator,
    /// Number of predicted characters displayed past the confirmed cursor.
    display_ahead: usize,
    /// Timestamp of the last input keystroke, for bulk paste detection.
    last_input_time: Option<Instant>,
    /// Bytes accumulated in the current input burst.
    input_byte_count: usize,
    /// Whether the terminal is in the alternate screen buffer (e.g., vim).
    /// Predictions are suppressed while this is `true`.
    in_alternate_screen: bool,
}

impl PredictionEngine {
    /// Creates a new prediction engine with the given mode.
    ///
    /// The terminal width defaults to 80 columns and can be updated via
    /// [`resize`](Self::resize).
    pub fn new(mode: PredictMode) -> Self {
        Self {
            mode,
            cursor_col: 0,
            term_width: 80,
            epoch_counter: 0,
            confirmed_epoch: 0,
            epochs: VecDeque::new(),
            rtt: RttEstimator::new(),
            display_ahead: 0,
            last_input_time: None,
            input_byte_count: 0,
            in_alternate_screen: false,
        }
    }

    /// Records a keystroke and returns the action the caller should take
    /// to display speculative local echo.
    ///
    /// Handles bulk paste detection (resets if >100 bytes arrive within 10 ms),
    /// printable character prediction, backspace prediction, and epoch boundaries
    /// for unpredictable input (Enter, Escape, control characters, escape
    /// sequences).
    pub fn on_keystroke(&mut self, encoded: &str) -> PredictionAction {
        let now = Instant::now();

        // Bulk paste detection: if bytes accumulate too fast, reset predictions.
        if let Some(last) = self.last_input_time {
            if now.duration_since(last) <= BULK_PASTE_WINDOW {
                self.input_byte_count = self.input_byte_count.saturating_add(encoded.len());
            } else {
                self.input_byte_count = encoded.len();
            }
        } else {
            self.input_byte_count = encoded.len();
        }
        self.last_input_time = Some(now);

        if self.input_byte_count > BULK_PASTE_THRESHOLD {
            trace!(
                "Bulk paste detected ({} bytes in burst), resetting predictions",
                self.input_byte_count
            );
            self.reset();
            return PredictionAction::None;
        }

        if !self.should_display() {
            return PredictionAction::None;
        }

        let bytes = encoded.as_bytes();

        // Single printable character.
        if encoded.len() == 1 {
            let ch = bytes[0] as char;

            // Backspace (BS or DEL).
            if bytes[0] == 0x08 || bytes[0] == 0x7f {
                if self.display_ahead > 0 {
                    // Undo our own pending forward prediction.
                    self.ensure_current_epoch();
                    let pred_col = self
                        .cursor_col
                        .saturating_add(self.display_ahead)
                        .saturating_sub(1);
                    if let Some(epoch) = self.epochs.back_mut() {
                        epoch.predictions.push_back(Prediction {
                            ch: '\x08',
                            col: pred_col,
                            sent_at: now,
                            state: PredictionState::Pending,
                            epoch: epoch.id,
                        });
                    }
                    self.display_ahead = self.display_ahead.saturating_sub(1);
                    trace!("Predicted backspace at col {}", pred_col);
                    return PredictionAction::DisplayBackspace;
                }
                // No pending forward predictions — treat as epoch boundary.
                // We cannot safely predict visual backspace because we don't
                // know where the prompt's editable region starts.
                self.new_epoch();
                trace!("Epoch boundary on backspace with no pending predictions");
                return PredictionAction::NewEpoch;
            }

            // Control characters and CR/LF trigger epoch boundaries.
            if ch == '\r' || ch == '\n' || ch == '\x1b' || ch.is_ascii_control() {
                self.new_epoch();
                trace!("Epoch boundary on control char {:02x}", bytes[0]);
                return PredictionAction::NewEpoch;
            }

            // Printable character.
            if ch.is_ascii_graphic() || ch == ' ' {
                self.ensure_current_epoch();
                let pred_col = self.cursor_col.saturating_add(self.display_ahead);
                if let Some(epoch) = self.epochs.back_mut() {
                    epoch.predictions.push_back(Prediction {
                        ch,
                        col: pred_col,
                        sent_at: now,
                        state: PredictionState::Pending,
                        epoch: epoch.id,
                    });
                }
                self.display_ahead = self.display_ahead.saturating_add(1);
                trace!("Predicted '{}' at col {}", ch, pred_col);
                return PredictionAction::DisplayChar(ch);
            }
        }

        // Multi-byte sequences (escape sequences, arrow keys, etc.) are
        // unpredictable — start a new epoch.
        self.new_epoch();
        trace!(
            "Epoch boundary on multi-byte input ({} bytes)",
            encoded.len()
        );
        PredictionAction::NewEpoch
    }

    /// Processes server output, confirming or rolling back predictions.
    ///
    /// Bytes that were already displayed speculatively are suppressed from
    /// `out`. Unmatched or unexpected bytes are written through. On mismatch
    /// in a confirmed epoch, a visual rollback is emitted and predictions
    /// are reset.
    pub fn process_server_output(&mut self, bytes: &[u8], out: &mut Vec<u8>) {
        let mut i = 0;
        while i < bytes.len() {
            // CSI sequence: ESC [ <params> <letter>
            if bytes[i] == 0x1b
                && i + 1 < bytes.len()
                && bytes[i + 1] == b'['
                && let Some((seq_len, cmd, param)) = self.parse_csi(&bytes[i..])
            {
                let csi_start = i;

                // Roll back predictions BEFORE writing the CSI so that the
                // cursor-left in rollback operates from the correct position.
                //
                // Cursor-movement or erase CSI while predictions are pending
                // means the server is doing something other than echoing
                // characters (e.g. vim cursor motion, screen clearing).
                if self.display_ahead > 0 {
                    match cmd {
                        b'A' | b'B' | b'C' | b'D' | b'G' | b'H' | b'J' | b'K' | b'd' => {
                            self.rollback(out);
                            self.reset();
                        }
                        _ => {}
                    }
                }

                // Alternate screen enter (DEC private modes 1049/47/1047):
                // rollback before the screen switch so cleanup targets the
                // main screen where predictions were displayed.
                let is_dec_alt_screen =
                    bytes[csi_start + 2] == b'?' && (param == 1049 || param == 47 || param == 1047);
                if is_dec_alt_screen && cmd == b'h' {
                    if self.display_ahead > 0 {
                        self.rollback(out);
                        self.reset();
                    }
                    self.in_alternate_screen = true;
                    trace!("Entered alternate screen (mode {})", param);
                }

                // Write the full sequence to output.
                out.extend_from_slice(&bytes[csi_start..csi_start + seq_len]);

                // Track cursor movement.
                match cmd {
                    b'C' => {
                        self.cursor_col = self.cursor_col.saturating_add(param);
                    }
                    b'D' => {
                        self.cursor_col = self.cursor_col.saturating_sub(param);
                    }
                    b'G' => {
                        self.cursor_col = param.saturating_sub(1);
                    }
                    _ => {}
                }

                // Alternate screen leave: clear flag after writing the CSI
                // (order doesn't matter for leave since we're restoring the
                // main screen, not cleaning up predictions).
                if is_dec_alt_screen && cmd == b'l' {
                    self.in_alternate_screen = false;
                    trace!("Left alternate screen (mode {})", param);
                }

                i += seq_len;
                continue;
            }

            let b = bytes[i];

            // Control characters that affect cursor position.
            match b {
                b'\r' => {
                    self.cursor_col = 0;
                    out.push(b);
                    i += 1;
                    continue;
                }
                b'\n' => {
                    self.cursor_col = 0;
                    out.push(b);
                    i += 1;
                    continue;
                }
                0x08 => {
                    // Backspace — confirm any pending BS prediction.
                    self.try_confirm_prediction('\x08');
                    self.cursor_col = self.cursor_col.saturating_sub(1);
                    out.push(b);
                    i += 1;
                    continue;
                }
                _ => {}
            }

            // Non-printable, non-tracked byte — pass through.
            if b < 0x20 || b == 0x7f {
                out.push(b);
                i += 1;
                continue;
            }

            // Printable character — attempt to match against predictions.
            let ch = b as char;
            // Note: if expire triggers rollback+reset, all predictions are
            // cleared and try_confirm_prediction returns None, so the byte
            // falls through to the pass-through path below.
            self.expire_old_predictions(out);

            if let Some(matched) = self.try_confirm_prediction(ch) {
                if matched {
                    if self.display_ahead > 0 {
                        // Overwrite the underlined prediction with the plain server byte.
                        out.extend_from_slice(format!("\x1b[{}D", self.display_ahead).as_bytes());
                        out.extend_from_slice(b"\x1b[24m");
                        out.push(b);
                        let remaining = self.display_ahead - 1;
                        if remaining > 0 {
                            out.extend_from_slice(format!("\x1b[{}C", remaining).as_bytes());
                        }
                        self.display_ahead -= 1;
                        trace!(
                            "Confirmed predicted '{}' at col {} (overwritten)",
                            ch, self.cursor_col
                        );
                    } else {
                        out.push(b);
                        trace!(
                            "Confirmed predicted '{}' at col {} (already consumed)",
                            ch, self.cursor_col
                        );
                    }
                } else {
                    // Mismatch — check if epoch was confirmed.
                    let mismatch_epoch = self.oldest_pending_epoch_id();
                    if let Some(eid) = mismatch_epoch {
                        if self.is_epoch_confirmed(eid) {
                            trace!("Mismatch in confirmed epoch {}, rolling back", eid);
                            self.rollback(out);
                            self.reset();
                        } else {
                            trace!("Mismatch in unconfirmed epoch {}, discarding", eid);
                            self.discard_epoch(eid, out);
                        }
                    }
                    out.push(b);
                }
            } else {
                // No pending predictions — pass through normally.
                out.push(b);
            }

            // Advance cursor for this printable character.
            self.cursor_col += 1;
            if self.cursor_col >= self.term_width {
                self.cursor_col = 0;
            }
            i += 1;
        }
    }

    /// Emits escape sequences to visually undo speculative display.
    ///
    /// If any characters were displayed ahead of the confirmed cursor,
    /// writes `ESC[{n}D` (cursor left) followed by `ESC[K` (clear to
    /// end of line) into `out`.
    pub fn rollback(&self, out: &mut Vec<u8>) {
        if self.display_ahead > 0 {
            trace!(
                "Rolling back {} display-ahead characters",
                self.display_ahead
            );
            // Reset SGR state (underline, color, etc.) before erasing.
            out.extend_from_slice(b"\x1b[0m");
            // Cursor left by display_ahead columns.
            out.extend_from_slice(format!("\x1b[{}D", self.display_ahead).as_bytes());
            // Clear to end of line.
            out.extend_from_slice(b"\x1b[K");
        }
    }

    /// Clears all prediction state.
    ///
    /// Removes all epochs, resets the display-ahead counter, and resets
    /// the epoch counter. Does not affect the RTT estimator or
    /// alternate-screen tracking.
    pub fn reset(&mut self) {
        trace!("Prediction engine reset");
        self.epochs.clear();
        self.display_ahead = 0;
        self.epoch_counter = 0;
        self.confirmed_epoch = 0;
    }

    /// Updates the terminal width used for cursor wrapping calculations.
    pub fn resize(&mut self, width: usize) {
        self.term_width = width;
    }

    /// Returns whether predictions should be displayed given the current
    /// mode and RTT.
    ///
    /// - `Off` mode: always returns `false`
    /// - `On` mode: always returns `true`
    /// - `Adaptive` mode: returns `true` when SRTT >= 30 ms
    pub fn should_display(&self) -> bool {
        if self.in_alternate_screen {
            return false;
        }
        match self.mode {
            PredictMode::Off => false,
            PredictMode::On => true,
            PredictMode::Adaptive => self.rtt.srtt() >= Duration::from_millis(30),
        }
    }

    /// Returns whether predicted characters should be underlined to
    /// visually distinguish them from confirmed output.
    ///
    /// Underlining activates when predictions are displayed and SRTT >= 80 ms,
    /// indicating latency high enough that the user benefits from a visual
    /// hint that characters are speculative.
    pub fn should_underline(&self) -> bool {
        self.should_display() && self.rtt.srtt() >= Duration::from_millis(80)
    }

    /// Returns whether the given epoch has had at least one prediction confirmed.
    pub fn is_epoch_confirmed(&self, epoch_id: u64) -> bool {
        epoch_id <= self.confirmed_epoch
    }

    /// Starts a new prediction epoch.
    fn new_epoch(&mut self) {
        self.epoch_counter += 1;
        self.epochs.push_back(Epoch {
            id: self.epoch_counter,
            predictions: VecDeque::new(),
        });
    }

    /// Ensures at least one epoch exists for recording predictions.
    fn ensure_current_epoch(&mut self) {
        if self.epochs.is_empty() {
            self.new_epoch();
        }
    }

    /// Parses a CSI sequence starting at `buf[0] == ESC`, `buf[1] == '['`.
    ///
    /// Returns `(total_length, command_byte, numeric_parameter)` on success.
    /// The numeric parameter defaults to 1 if absent. Returns `None` for
    /// incomplete or malformed sequences.
    fn parse_csi(&self, buf: &[u8]) -> Option<(usize, u8, usize)> {
        if buf.len() < 3 || buf[0] != 0x1b || buf[1] != b'[' {
            return None;
        }

        let mut j = 2;
        // Collect digits and semicolons (intermediate bytes).
        while j < buf.len() && (buf[j].is_ascii_digit() || buf[j] == b';' || buf[j] == b'?') {
            j += 1;
        }

        // Need at least the final command byte.
        if j >= buf.len() {
            return None;
        }

        let cmd = buf[j];
        // CSI command bytes are in the range 0x40..=0x7E.
        if !(0x40..=0x7E).contains(&cmd) {
            return None;
        }

        // Extract numeric parameter (first number before any semicolon).
        let param_slice = &buf[2..j];
        let param = if param_slice.is_empty() {
            1
        } else {
            // Find first number (before any '?' prefix or ';' separator).
            let start = if param_slice[0] == b'?' { 1 } else { 0 };
            let end = param_slice
                .iter()
                .position(|&b| b == b';')
                .unwrap_or(param_slice.len());
            if start >= end {
                1
            } else {
                // Safety: we checked that all bytes in range are ASCII digits, ';', or '?'.
                std::str::from_utf8(&param_slice[start..end])
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
            }
        };

        Some((j + 1, cmd, param))
    }

    /// Attempts to confirm the oldest pending prediction against a received character.
    ///
    /// Returns `Some(true)` if the prediction matches, `Some(false)` on mismatch,
    /// or `None` if there are no pending predictions.
    fn try_confirm_prediction(&mut self, ch: char) -> Option<bool> {
        for epoch in &mut self.epochs {
            for pred in &mut epoch.predictions {
                if pred.state == PredictionState::Pending {
                    if pred.ch == ch {
                        pred.state = PredictionState::Confirmed;
                        let rtt_sample = pred.sent_at.elapsed();
                        self.rtt.update(rtt_sample);
                        self.confirmed_epoch = self.confirmed_epoch.max(epoch.id);
                        trace!(
                            "RTT sample: {:?} (srtt now {:?})",
                            rtt_sample,
                            self.rtt.srtt()
                        );
                        return Some(true);
                    }
                    return Some(false);
                }
            }
        }
        None
    }

    /// Returns the epoch ID of the oldest pending prediction, if any.
    fn oldest_pending_epoch_id(&self) -> Option<u64> {
        for epoch in &self.epochs {
            for pred in &epoch.predictions {
                if pred.state == PredictionState::Pending {
                    return Some(epoch.id);
                }
            }
        }
        None
    }

    /// Discards all predictions in the given epoch, adjusting `display_ahead`
    /// and emitting rollback sequences for any speculatively displayed characters.
    fn discard_epoch(&mut self, epoch_id: u64, out: &mut Vec<u8>) {
        // Count pending non-backspace predictions that were displayed ahead.
        let displayed = self
            .epochs
            .iter()
            .filter(|e| e.id == epoch_id)
            .flat_map(|e| &e.predictions)
            .filter(|p| p.state == PredictionState::Pending && p.ch != '\x08')
            .count();
        if displayed > 0 && self.display_ahead >= displayed {
            // Emit rollback for the discarded epoch's displayed chars.
            out.extend_from_slice(b"\x1b[0m");
            out.extend_from_slice(format!("\x1b[{}D", displayed).as_bytes());
            out.extend_from_slice(b"\x1b[K");
            self.display_ahead -= displayed;
        }
        self.epochs.retain(|e| e.id != epoch_id);
    }

    /// Marks predictions older than [`PREDICTION_EXPIRY`] as failed.
    ///
    /// If any non-backspace predictions expire while characters are displayed
    /// ahead, the speculative display is rolled back and the engine is reset.
    fn expire_old_predictions(&mut self, out: &mut Vec<u8>) {
        let now = Instant::now();
        let mut expired_display = 0usize;
        for epoch in &mut self.epochs {
            for pred in &mut epoch.predictions {
                if pred.state == PredictionState::Pending
                    && now.duration_since(pred.sent_at) > PREDICTION_EXPIRY
                {
                    trace!("Prediction '{}' at col {} expired", pred.ch, pred.col);
                    pred.state = PredictionState::Failed;
                    if pred.ch != '\x08' {
                        expired_display += 1;
                    }
                }
            }
        }
        if expired_display > 0 && self.display_ahead > 0 {
            self.rollback(out);
            self.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine_on() -> PredictionEngine {
        PredictionEngine::new(PredictMode::On)
    }

    fn engine_off() -> PredictionEngine {
        PredictionEngine::new(PredictMode::Off)
    }

    fn engine_adaptive() -> PredictionEngine {
        PredictionEngine::new(PredictMode::Adaptive)
    }

    #[test]
    fn printable_char_should_return_display_char() {
        let mut engine = engine_on();
        let action = engine.on_keystroke("a");
        assert_eq!(action, PredictionAction::DisplayChar('a'));
    }

    #[test]
    fn printable_space_should_return_display_char() {
        let mut engine = engine_on();
        let action = engine.on_keystroke(" ");
        assert_eq!(action, PredictionAction::DisplayChar(' '));
    }

    #[test]
    fn printable_digit_should_return_display_char() {
        let mut engine = engine_on();
        let action = engine.on_keystroke("5");
        assert_eq!(action, PredictionAction::DisplayChar('5'));
    }

    #[test]
    fn printable_symbol_should_return_display_char() {
        let mut engine = engine_on();
        let action = engine.on_keystroke("@");
        assert_eq!(action, PredictionAction::DisplayChar('@'));
    }

    #[test]
    fn multiple_printable_chars_should_each_return_display_char() {
        let mut engine = engine_on();
        assert_eq!(engine.on_keystroke("h"), PredictionAction::DisplayChar('h'));
        assert_eq!(engine.on_keystroke("i"), PredictionAction::DisplayChar('i'));
        assert_eq!(engine.on_keystroke("!"), PredictionAction::DisplayChar('!'));
    }

    #[test]
    fn printable_char_should_increment_display_ahead() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        engine.on_keystroke("b");
        engine.on_keystroke("c");
        assert_eq!(engine.display_ahead, 3);
    }

    #[test]
    fn confirmation_should_overwrite_predicted_char() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        let mut out = Vec::new();
        engine.process_server_output(b"a", &mut out);
        // With display_ahead=1: cursor back 1, underline off, the confirmed char.
        // No cursor-forward since remaining display_ahead is 0.
        assert_eq!(out, b"\x1b[1D\x1b[24ma");
    }

    #[test]
    fn confirmation_should_decrement_display_ahead() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        engine.on_keystroke("b");
        assert_eq!(engine.display_ahead, 2);

        let mut out = Vec::new();
        // Confirm "a" (display_ahead=2): overwrite sequence emitted.
        engine.process_server_output(b"a", &mut out);
        assert_eq!(engine.display_ahead, 1);
        assert_eq!(out, b"\x1b[2D\x1b[24ma\x1b[1C");

        out.clear();
        // Confirm "b" (display_ahead=1): overwrite sequence, no cursor-forward.
        engine.process_server_output(b"b", &mut out);
        assert_eq!(engine.display_ahead, 0);
        assert_eq!(out, b"\x1b[1D\x1b[24mb");
    }

    #[test]
    fn confirmation_should_advance_cursor_col() {
        let mut engine = engine_on();
        engine.on_keystroke("x");
        let mut out = Vec::new();
        engine.process_server_output(b"x", &mut out);
        assert_eq!(engine.cursor_col, 1);
    }

    #[test]
    fn unconfirmed_output_should_pass_through() {
        let mut engine = engine_on();
        // No predictions — raw server output should pass through.
        let mut out = Vec::new();
        engine.process_server_output(b"hello", &mut out);
        assert_eq!(out, b"hello");
    }

    #[test]
    fn backspace_with_no_display_ahead_should_return_new_epoch() {
        let mut engine = engine_on();
        // cursor_col advanced via server output, but display_ahead remains 0.
        let mut out = Vec::new();
        engine.process_server_output(b"abc", &mut out);
        assert_eq!(engine.cursor_col, 3);

        let action = engine.on_keystroke("\x7f");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn backspace_bs_byte_with_no_display_ahead_should_return_new_epoch() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"x", &mut out);
        assert_eq!(engine.cursor_col, 1);

        let action = engine.on_keystroke("\x08");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn backspace_at_col_zero_with_no_display_ahead_should_return_new_epoch() {
        let mut engine = engine_on();
        // cursor_col starts at 0, no chars typed — display_ahead is 0.
        let action = engine.on_keystroke("\x7f");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn backspace_should_decrement_display_ahead() {
        let mut engine = engine_on();
        // Advance confirmed cursor so backspace is permitted.
        let mut out = Vec::new();
        engine.process_server_output(b"xy", &mut out);
        assert_eq!(engine.cursor_col, 2);

        engine.on_keystroke("a");
        engine.on_keystroke("b");
        assert_eq!(engine.display_ahead, 2);
        engine.on_keystroke("\x7f");
        assert_eq!(engine.display_ahead, 1);
    }

    #[test]
    fn enter_should_return_new_epoch() {
        let mut engine = engine_on();
        let action = engine.on_keystroke("\r");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn linefeed_should_return_new_epoch() {
        let mut engine = engine_on();
        let action = engine.on_keystroke("\n");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn escape_should_return_new_epoch() {
        let mut engine = engine_on();
        let action = engine.on_keystroke("\x1b");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn ctrl_c_should_return_new_epoch() {
        let mut engine = engine_on();
        let action = engine.on_keystroke("\x03");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn ctrl_d_should_return_new_epoch() {
        let mut engine = engine_on();
        let action = engine.on_keystroke("\x04");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn tab_should_return_new_epoch() {
        let mut engine = engine_on();
        let action = engine.on_keystroke("\t");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn multi_byte_escape_sequence_should_return_new_epoch() {
        let mut engine = engine_on();
        // Arrow key: ESC [ A
        let action = engine.on_keystroke("\x1b[A");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn epoch_counter_should_increment_on_epoch_boundary() {
        let mut engine = engine_on();
        engine.on_keystroke("a"); // Creates epoch 1 implicitly
        assert_eq!(engine.epoch_counter, 1);
        engine.on_keystroke("\r"); // Creates epoch 2
        assert_eq!(engine.epoch_counter, 2);
        engine.on_keystroke("\r"); // Creates epoch 3
        assert_eq!(engine.epoch_counter, 3);
    }

    #[test]
    fn predictions_in_unconfirmed_epoch_should_be_discarded_on_mismatch() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        // Server sends a different character — mismatch in unconfirmed epoch.
        let mut out = Vec::new();
        engine.process_server_output(b"z", &mut out);
        // The discard_epoch rollback emits SGR reset + cursor-back 1 + erase, then 'z' passes through.
        assert_eq!(out, b"\x1b[0m\x1b[1D\x1b[Kz");
        // The epoch with 'a' should have been discarded.
        let has_pending = engine.epochs.iter().any(|e| {
            e.predictions
                .iter()
                .any(|p| p.state == PredictionState::Pending)
        });
        assert!(
            !has_pending,
            "unconfirmed epoch predictions should be discarded on mismatch"
        );
    }

    #[test]
    fn confirmed_epoch_mismatch_should_trigger_rollback() {
        let mut engine = engine_on();
        // Type "abc" to establish predictions (display_ahead=3).
        engine.on_keystroke("a");
        engine.on_keystroke("b");
        engine.on_keystroke("c");

        let mut out = Vec::new();
        // Confirm "a" — this marks the epoch as confirmed.
        // With display_ahead=3: overwrite = ESC[3D ESC[24m 'a' ESC[2C
        engine.process_server_output(b"a", &mut out);
        assert_eq!(out, b"\x1b[3D\x1b[24ma\x1b[2C");
        out.clear();

        // Confirm "b" (display_ahead=2): overwrite = ESC[2D ESC[24m 'b' ESC[1C
        engine.process_server_output(b"b", &mut out);
        assert_eq!(out, b"\x1b[2D\x1b[24mb\x1b[1C");
        out.clear();

        // Now server sends "z" instead of "c" — mismatch in confirmed epoch.
        // display_ahead=1, rollback emits SGR reset + cursor-left 1 + clear, then 'z'.
        engine.process_server_output(b"z", &mut out);
        assert_eq!(out, b"\x1b[0m\x1b[1D\x1b[Kz");
    }

    #[test]
    fn rollback_should_emit_sgr_reset_cursor_left_and_clear() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        engine.on_keystroke("b");
        engine.on_keystroke("c");
        assert_eq!(engine.display_ahead, 3);

        let mut out = Vec::new();
        engine.rollback(&mut out);
        // Should contain ESC[0m (SGR reset), ESC[3D (cursor left 3), and ESC[K (clear to EOL).
        assert_eq!(out, b"\x1b[0m\x1b[3D\x1b[K");
    }

    #[test]
    fn rollback_with_zero_display_ahead_should_emit_nothing() {
        let engine = engine_on();
        let mut out = Vec::new();
        engine.rollback(&mut out);
        assert!(
            out.is_empty(),
            "rollback with no display-ahead should be empty"
        );
    }

    #[test]
    fn rollback_with_one_display_ahead_should_emit_one_column() {
        let mut engine = engine_on();
        engine.on_keystroke("x");
        let mut out = Vec::new();
        engine.rollback(&mut out);
        assert_eq!(out, b"\x1b[0m\x1b[1D\x1b[K");
    }

    #[test]
    fn rtt_update_should_converge_toward_samples() {
        let mut rtt = RttEstimator::new();
        // Initial SRTT is 100ms. Feed it 20ms samples.
        for _ in 0..50 {
            rtt.update(Duration::from_millis(20));
        }
        let final_srtt = rtt.srtt();
        // After many iterations, SRTT should be close to 20ms.
        assert!(
            final_srtt < Duration::from_millis(25),
            "SRTT should converge to ~20ms, got {:?}",
            final_srtt
        );
        assert!(
            final_srtt >= Duration::from_millis(18),
            "SRTT should not undershoot 18ms, got {:?}",
            final_srtt
        );
    }

    #[test]
    fn rtt_update_should_converge_toward_high_samples() {
        let mut rtt = RttEstimator::new();
        // Initial SRTT is 100ms. Feed it 500ms samples.
        for _ in 0..50 {
            rtt.update(Duration::from_millis(500));
        }
        let final_srtt = rtt.srtt();
        assert!(
            final_srtt > Duration::from_millis(490),
            "SRTT should converge to ~500ms, got {:?}",
            final_srtt
        );
        assert!(
            final_srtt <= Duration::from_millis(510),
            "SRTT should not overshoot 510ms, got {:?}",
            final_srtt
        );
    }

    #[test]
    fn rtt_new_should_have_default_values() {
        let rtt = RttEstimator::new();
        assert_eq!(rtt.srtt(), Duration::from_millis(100));
    }

    #[test]
    fn rtt_default_should_match_new() {
        let rtt_new = RttEstimator::new();
        let rtt_default = RttEstimator::default();
        assert_eq!(rtt_new.srtt(), rtt_default.srtt());
    }

    #[test]
    fn rtt_single_update_should_blend_with_initial() {
        let mut rtt = RttEstimator::new();
        // srtt = 7/8 * 100 + 1/8 * 200 = 87.5 + 25 = 112.5ms
        rtt.update(Duration::from_millis(200));
        let srtt_ms = rtt.srtt().as_millis();
        assert_eq!(srtt_ms, 112, "single update should blend 7/8 old + 1/8 new");
    }

    #[test]
    fn rtt_update_with_zero_sample_should_decrease_srtt() {
        let mut rtt = RttEstimator::new();
        rtt.update(Duration::ZERO);
        // 7/8 * 100 + 1/8 * 0 = 87.5ms, truncated to 87ms.
        assert_eq!(
            rtt.srtt().as_millis(),
            87,
            "zero sample should blend to 87ms"
        );
    }

    #[test]
    fn rtt_update_with_max_duration_should_saturate_without_overflow() {
        let mut rtt = RttEstimator::new();
        rtt.update(Duration::MAX);
        // Saturating arithmetic: 7/8 * 100ms is small, 1/8 * MAX is enormous.
        // Result should be near Duration::MAX / 8.
        let srtt = rtt.srtt();
        assert!(
            srtt > Duration::from_secs(1_000_000),
            "SRTT should be very large after MAX sample, got {:?}",
            srtt
        );
    }

    #[test]
    fn csi_cursor_forward_should_advance_cursor() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // ESC[5C = cursor forward 5
        engine.process_server_output(b"\x1b[5C", &mut out);
        assert_eq!(engine.cursor_col, 5);
        assert_eq!(out, b"\x1b[5C", "CSI sequence should be passed through");
    }

    #[test]
    fn csi_cursor_backward_should_retreat_cursor() {
        let mut engine = engine_on();
        engine.cursor_col = 10;
        let mut out = Vec::new();
        // ESC[3D = cursor backward 3
        engine.process_server_output(b"\x1b[3D", &mut out);
        assert_eq!(engine.cursor_col, 7);
        assert_eq!(out, b"\x1b[3D");
    }

    #[test]
    fn csi_cursor_backward_should_not_underflow() {
        let mut engine = engine_on();
        engine.cursor_col = 2;
        let mut out = Vec::new();
        engine.process_server_output(b"\x1b[10D", &mut out);
        assert_eq!(engine.cursor_col, 0);
    }

    #[test]
    fn csi_cursor_horizontal_absolute_should_set_cursor() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // ESC[15G = cursor to column 15 (1-based), so 0-based = 14
        engine.process_server_output(b"\x1b[15G", &mut out);
        assert_eq!(engine.cursor_col, 14);
        assert_eq!(out, b"\x1b[15G");
    }

    #[test]
    fn csi_cursor_horizontal_absolute_default_should_go_to_col_zero() {
        let mut engine = engine_on();
        engine.cursor_col = 10;
        let mut out = Vec::new();
        // ESC[G with no parameter defaults to 1 (1-based), so 0-based = 0
        engine.process_server_output(b"\x1b[G", &mut out);
        assert_eq!(engine.cursor_col, 0);
    }

    #[test]
    fn csi_cursor_forward_default_param_should_advance_by_one() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // ESC[C with no parameter defaults to 1
        engine.process_server_output(b"\x1b[C", &mut out);
        assert_eq!(engine.cursor_col, 1);
    }

    #[test]
    fn cr_should_reset_cursor_to_zero() {
        let mut engine = engine_on();
        engine.cursor_col = 15;
        let mut out = Vec::new();
        engine.process_server_output(b"\r", &mut out);
        assert_eq!(engine.cursor_col, 0);
        assert_eq!(out, b"\r");
    }

    #[test]
    fn lf_should_reset_cursor_to_zero() {
        let mut engine = engine_on();
        engine.cursor_col = 15;
        let mut out = Vec::new();
        engine.process_server_output(b"\n", &mut out);
        assert_eq!(engine.cursor_col, 0);
        assert_eq!(out, b"\n");
    }

    #[test]
    fn bs_in_server_output_should_decrement_cursor() {
        let mut engine = engine_on();
        engine.cursor_col = 5;
        let mut out = Vec::new();
        engine.process_server_output(b"\x08", &mut out);
        assert_eq!(engine.cursor_col, 4);
        assert_eq!(out, b"\x08");
    }

    #[test]
    fn bs_in_server_output_at_zero_should_not_underflow() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"\x08", &mut out);
        assert_eq!(engine.cursor_col, 0);
    }

    #[test]
    fn mixed_text_and_csi_should_track_cursor_correctly() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // "abc" (cursor at 3), then ESC[2D (back to 1), then "x" (cursor at 2)
        engine.process_server_output(b"abc\x1b[2Dx", &mut out);
        assert_eq!(engine.cursor_col, 2);
        assert_eq!(out, b"abc\x1b[2Dx");
    }

    #[test]
    fn cursor_should_wrap_at_term_width() {
        let mut engine = engine_on();
        engine.resize(5);
        let mut out = Vec::new();
        // Type 5 chars — cursor should wrap to col 0 on the 5th.
        engine.process_server_output(b"abcde", &mut out);
        assert_eq!(engine.cursor_col, 0, "cursor should wrap at term_width=5");
    }

    #[test]
    fn bulk_paste_should_return_none_and_reset() {
        let mut engine = engine_on();
        // Feed > 100 single-byte keystrokes in a tight loop (no sleep).
        // Since this is single-threaded and Instant::now() won't advance
        // much, the 10ms window should include all of them.
        let mut actions = Vec::new();
        for i in 0..=BULK_PASTE_THRESHOLD {
            let ch = (b'a' + (i % 26) as u8) as char;
            actions.push(engine.on_keystroke(&ch.to_string()));
        }
        // The last action (after threshold is exceeded) should be None.
        assert_eq!(
            *actions.last().unwrap(),
            PredictionAction::None,
            "bulk paste should trigger None after threshold"
        );
        // Engine should be reset: no epochs, no display_ahead.
        assert!(
            engine.epochs.is_empty(),
            "epochs should be cleared on bulk paste reset"
        );
        assert_eq!(engine.display_ahead, 0);
    }

    #[test]
    fn should_display_off_mode_should_return_false() {
        let engine = engine_off();
        assert!(!engine.should_display());
    }

    #[test]
    fn should_display_on_mode_should_return_true() {
        let engine = engine_on();
        assert!(engine.should_display());
    }

    #[test]
    fn should_display_adaptive_mode_should_return_true_with_high_rtt() {
        let mut engine = engine_adaptive();
        // Default SRTT is 100ms, which is >= 30ms threshold.
        assert!(
            engine.should_display(),
            "adaptive mode should display when SRTT (100ms) >= 30ms"
        );
        // Now feed many low-RTT samples to bring SRTT below 30ms.
        for _ in 0..100 {
            engine.rtt.update(Duration::from_millis(1));
        }
        assert!(
            !engine.should_display(),
            "adaptive mode should not display when SRTT < 30ms"
        );
    }

    #[test]
    fn should_display_adaptive_mode_should_return_false_with_low_rtt() {
        let mut engine = engine_adaptive();
        // Drive SRTT well below 30ms.
        for _ in 0..100 {
            engine.rtt.update(Duration::from_millis(1));
        }
        assert!(!engine.should_display());
    }

    #[test]
    fn on_keystroke_off_mode_should_return_none() {
        let mut engine = engine_off();
        let action = engine.on_keystroke("a");
        assert_eq!(action, PredictionAction::None);
    }

    #[test]
    fn on_keystroke_adaptive_low_rtt_should_return_none() {
        let mut engine = engine_adaptive();
        for _ in 0..100 {
            engine.rtt.update(Duration::from_millis(1));
        }
        let action = engine.on_keystroke("a");
        assert_eq!(action, PredictionAction::None);
    }

    #[test]
    fn should_underline_should_require_high_rtt() {
        let engine = engine_on();
        // Default SRTT is 100ms >= 80ms threshold.
        assert!(engine.should_underline());
    }

    #[test]
    fn should_underline_should_return_false_for_moderate_rtt() {
        let mut engine = engine_on();
        // Drive SRTT to ~50ms (above 30ms display threshold but below 80ms underline).
        for _ in 0..100 {
            engine.rtt.update(Duration::from_millis(50));
        }
        assert!(engine.should_display(), "50ms SRTT should display");
        assert!(!engine.should_underline(), "50ms SRTT should not underline");
    }

    #[test]
    fn should_underline_off_mode_should_return_false() {
        let engine = engine_off();
        assert!(!engine.should_underline());
    }

    #[test]
    fn reset_should_clear_all_state() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        engine.on_keystroke("b");
        engine.on_keystroke("c");
        assert!(!engine.epochs.is_empty());
        assert_eq!(engine.display_ahead, 3);
        assert!(engine.epoch_counter > 0);

        engine.reset();
        assert!(engine.epochs.is_empty());
        assert_eq!(engine.display_ahead, 0);
        assert_eq!(engine.epoch_counter, 0);
        assert_eq!(engine.confirmed_epoch, 0);
    }

    #[test]
    fn reset_should_not_affect_rtt() {
        let mut engine = engine_on();
        engine.rtt.update(Duration::from_millis(200));
        let srtt_before = engine.rtt.srtt();
        engine.reset();
        assert_eq!(engine.rtt.srtt(), srtt_before);
    }

    #[test]
    fn resize_should_update_term_width() {
        let mut engine = engine_on();
        assert_eq!(engine.term_width, 80);
        engine.resize(120);
        assert_eq!(engine.term_width, 120);
    }

    #[test]
    fn is_epoch_confirmed_should_return_false_for_new_epoch() {
        let mut engine = engine_on();
        engine.on_keystroke("a"); // Creates epoch 1.
        assert!(
            !engine.is_epoch_confirmed(1),
            "epoch should not be confirmed before any server output"
        );
    }

    #[test]
    fn is_epoch_confirmed_should_return_true_after_confirmation() {
        let mut engine = engine_on();
        engine.on_keystroke("a"); // Creates epoch 1.
        let mut out = Vec::new();
        engine.process_server_output(b"a", &mut out);
        assert!(
            engine.is_epoch_confirmed(1),
            "epoch should be confirmed after matching server output"
        );
    }

    #[test]
    fn non_printable_server_bytes_should_pass_through() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // Bell (0x07) and other non-printable, non-tracked bytes.
        engine.process_server_output(b"\x07\x0e\x0f", &mut out);
        assert_eq!(out, b"\x07\x0e\x0f");
    }

    #[test]
    fn csi_with_question_mark_prefix_should_be_parsed() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // ESC[?25h (show cursor) — should be passed through and parsed without crash.
        engine.process_server_output(b"\x1b[?25h", &mut out);
        assert_eq!(out, b"\x1b[?25h");
    }

    #[test]
    fn csi_with_semicolons_should_be_parsed() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // ESC[1;31m (SGR bold red) — should pass through.
        engine.process_server_output(b"\x1b[1;31m", &mut out);
        assert_eq!(out, b"\x1b[1;31m");
    }

    #[test]
    fn incomplete_csi_at_end_should_pass_through() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // Incomplete CSI (ESC followed by end of buffer).
        engine.process_server_output(b"\x1b", &mut out);
        assert_eq!(out, b"\x1b");
    }

    #[test]
    fn multiple_csi_sequences_should_all_be_tracked() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // Move forward 5, then back 2: net cursor at 3.
        engine.process_server_output(b"\x1b[5C\x1b[2D", &mut out);
        assert_eq!(engine.cursor_col, 3);
        assert_eq!(out, b"\x1b[5C\x1b[2D");
    }

    #[test]
    fn prediction_sequence_type_then_confirm_all() {
        let mut engine = engine_on();
        engine.on_keystroke("h");
        engine.on_keystroke("e");
        engine.on_keystroke("l");
        engine.on_keystroke("l");
        engine.on_keystroke("o");

        let mut out = Vec::new();
        engine.process_server_output(b"hello", &mut out);
        // Each confirmed char emits an overwrite sequence:
        //   'h' at da=5: ESC[5D ESC[24m h ESC[4C
        //   'e' at da=4: ESC[4D ESC[24m e ESC[3C
        //   'l' at da=3: ESC[3D ESC[24m l ESC[2C
        //   'l' at da=2: ESC[2D ESC[24m l ESC[1C
        //   'o' at da=1: ESC[1D ESC[24m o (no cursor-forward, remaining=0)
        let mut expected = Vec::new();
        expected.extend_from_slice(b"\x1b[5D\x1b[24mh\x1b[4C");
        expected.extend_from_slice(b"\x1b[4D\x1b[24me\x1b[3C");
        expected.extend_from_slice(b"\x1b[3D\x1b[24ml\x1b[2C");
        expected.extend_from_slice(b"\x1b[2D\x1b[24ml\x1b[1C");
        expected.extend_from_slice(b"\x1b[1D\x1b[24mo");
        assert_eq!(out, expected);
        assert_eq!(engine.display_ahead, 0);
    }

    #[test]
    fn backspace_at_cursor_col_greater_than_zero_with_no_display_ahead_should_return_new_epoch() {
        let mut engine = engine_on();
        // Simulate some confirmed server output to move cursor forward.
        let mut out = Vec::new();
        engine.process_server_output(b"abc", &mut out);
        assert_eq!(engine.cursor_col, 3);

        // display_ahead is 0 — backspace treated as epoch boundary.
        let action = engine.on_keystroke("\x7f");
        assert_eq!(action, PredictionAction::NewEpoch);
    }

    #[test]
    fn predict_mode_default_should_be_adaptive() {
        let mode = PredictMode::default();
        assert_eq!(mode, PredictMode::Adaptive);
    }

    #[test]
    fn prediction_state_should_start_as_pending() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        let pred = &engine.epochs.back().unwrap().predictions[0];
        assert_eq!(pred.state, PredictionState::Pending);
        assert_eq!(pred.ch, 'a');
        assert_eq!(pred.col, 0);
    }

    #[test]
    fn prediction_state_should_transition_to_confirmed() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        let mut out = Vec::new();
        engine.process_server_output(b"a", &mut out);
        let pred = &engine.epochs.back().unwrap().predictions[0];
        assert_eq!(pred.state, PredictionState::Confirmed);
    }

    #[test]
    fn new_engine_should_have_default_state() {
        let engine = engine_on();
        assert_eq!(engine.cursor_col, 0);
        assert_eq!(engine.term_width, 80);
        assert_eq!(engine.epoch_counter, 0);
        assert_eq!(engine.confirmed_epoch, 0);
        assert!(engine.epochs.is_empty());
        assert_eq!(engine.display_ahead, 0);
        assert!(engine.last_input_time.is_none());
        assert_eq!(engine.input_byte_count, 0);
    }

    #[test]
    fn del_byte_should_pass_through_in_server_output() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"\x7f", &mut out);
        assert_eq!(out, b"\x7f");
    }

    #[test]
    fn csi_unrelated_commands_should_pass_through_without_cursor_change() {
        let mut engine = engine_on();
        engine.cursor_col = 5;
        let mut out = Vec::new();
        // ESC[2J = erase display — not a cursor movement tracked by engine.
        engine.process_server_output(b"\x1b[2J", &mut out);
        assert_eq!(
            engine.cursor_col, 5,
            "non-cursor CSI should not change cursor_col"
        );
        assert_eq!(out, b"\x1b[2J");
    }

    #[test]
    fn epoch_boundary_then_printable_should_create_separate_epochs() {
        let mut engine = engine_on();
        engine.on_keystroke("a"); // Epoch 1
        engine.on_keystroke("\r"); // Creates epoch 2 (boundary)
        engine.on_keystroke("b"); // Goes into epoch 2

        assert_eq!(engine.epochs.len(), 2, "should have exactly 2 epochs");
        // Epoch 1 should contain 'a'.
        assert_eq!(engine.epochs[0].predictions.len(), 1);
        assert_eq!(engine.epochs[0].predictions[0].ch, 'a');
        // Epoch 2 should contain 'b' (enter created the epoch, b was added to it).
        assert_eq!(engine.epochs[1].predictions.len(), 1);
        assert_eq!(engine.epochs[1].predictions[0].ch, 'b');
    }

    #[test]
    fn rtt_variance_should_decrease_with_consistent_samples() {
        let mut rtt = RttEstimator::new();
        // Feed consistent 50ms samples.
        for _ in 0..50 {
            rtt.update(Duration::from_millis(50));
        }
        // Variance should be very small since samples are identical.
        assert!(
            rtt.rttvar < Duration::from_millis(5),
            "variance should be small with consistent samples, got {:?}",
            rtt.rttvar
        );
    }

    #[test]
    fn rtt_variance_should_be_larger_with_variable_samples() {
        let mut rtt_consistent = RttEstimator::new();
        let mut rtt_variable = RttEstimator::new();

        for _ in 0..50 {
            rtt_consistent.update(Duration::from_millis(50));
        }

        for i in 0..50 {
            let sample = if i % 2 == 0 { 20 } else { 200 };
            rtt_variable.update(Duration::from_millis(sample));
        }

        assert!(
            rtt_variable.rttvar > rtt_consistent.rttvar,
            "variable samples should produce higher variance: consistent={:?}, variable={:?}",
            rtt_consistent.rttvar,
            rtt_variable.rttvar
        );
    }

    #[test]
    fn expire_old_predictions_should_trigger_rollback_and_reset() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        assert_eq!(engine.display_ahead, 1);

        // Manually backdate the prediction to be older than PREDICTION_EXPIRY.
        if let Some(epoch) = engine.epochs.back_mut() {
            epoch.predictions[0].sent_at =
                Instant::now() - PREDICTION_EXPIRY - Duration::from_millis(100);
        }

        // Trigger expiry check by processing server output with a printable char.
        let mut out = Vec::new();
        engine.process_server_output(b"x", &mut out);

        // Expiry triggers rollback (SGR reset + cursor-left 1 + clear) then reset clears
        // all epochs. The 'x' passes through since no pending predictions remain.
        assert_eq!(out, b"\x1b[0m\x1b[1D\x1b[Kx");
        assert!(
            engine.epochs.is_empty(),
            "epochs should be empty after expire-triggered reset"
        );
        assert_eq!(engine.display_ahead, 0);
    }

    #[test]
    fn confirm_then_subsequent_chars_should_pass_through() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        let mut out = Vec::new();
        engine.process_server_output(b"a", &mut out);
        // Confirmation produces overwrite output (display_ahead was 1).
        assert_eq!(out, b"\x1b[1D\x1b[24ma");
        assert_eq!(engine.display_ahead, 0);

        // Now server sends "bc" with no predictions — should pass through.
        out.clear();
        engine.process_server_output(b"bc", &mut out);
        assert_eq!(out, b"bc");
    }

    #[test]
    fn crlf_in_server_output_should_reset_cursor() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"hello\r\n", &mut out);
        assert_eq!(engine.cursor_col, 0);
        assert_eq!(out, b"hello\r\n");
    }

    #[test]
    fn printable_chars_after_cr_should_track_correctly() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"hello\rworld", &mut out);
        // After CR, cursor is at 0. "world" is 5 chars, cursor at 5.
        assert_eq!(engine.cursor_col, 5);
    }

    #[test]
    fn bulk_paste_below_threshold_should_still_predict() {
        let mut engine = engine_on();
        // Feed exactly at threshold — should still predict.
        for i in 0..BULK_PASTE_THRESHOLD {
            let ch = (b'a' + (i % 26) as u8) as char;
            let action = engine.on_keystroke(&ch.to_string());
            assert_ne!(
                action,
                PredictionAction::None,
                "should still predict at byte count {}",
                i + 1
            );
        }
    }

    #[test]
    fn backspace_prediction_should_record_correct_col() {
        let mut engine = engine_on();
        // Advance confirmed cursor to allow backspace.
        let mut out = Vec::new();
        engine.process_server_output(b"xyz", &mut out);
        assert_eq!(engine.cursor_col, 3);

        engine.on_keystroke("a"); // display_ahead=1, pred col=3
        engine.on_keystroke("b"); // display_ahead=2, pred col=4
        engine.on_keystroke("\x7f"); // backspace: pred col = 3 + 2 - 1 = 4

        let epoch = engine.epochs.back().unwrap();
        let bs_pred = epoch.predictions.back().unwrap();
        assert_eq!(bs_pred.ch, '\x08');
        // cursor_col(3) + display_ahead(2) - 1 = 4
        assert_eq!(bs_pred.col, 4);
    }

    #[test]
    fn csi_with_invalid_command_byte_should_pass_through_raw_bytes() {
        let mut engine = engine_on();
        engine.cursor_col = 5;
        let mut out = Vec::new();
        // ESC[5<space> — space (0x20) is outside valid CSI command range 0x40..=0x7E.
        engine.process_server_output(b"\x1b[5 ", &mut out);
        // Should pass through byte-by-byte (not parsed as CSI).
        assert_eq!(out, b"\x1b[5 ");
        // '5' and ' ' are printable, so cursor advances; ESC and '[' are control/non-printable.
        // ESC (0x1b) and '[' (0x5b — printable!) are processed individually.
        // The key point: cursor_col should NOT have been affected by CSI movement.
        // '[' at 0x5b is printable → cursor advances from 5 to 6
        // '5' is printable → cursor advances from 6 to 7
        // ' ' is printable → cursor advances from 7 to 8
        assert_ne!(
            engine.cursor_col, 5,
            "raw bytes should be processed individually, affecting cursor"
        );
    }

    #[test]
    fn backspace_prediction_should_be_confirmed_by_server_bs() {
        let mut engine = engine_on();
        // Type a char to get display_ahead=1, then backspace undoes it.
        let action = engine.on_keystroke("a");
        assert_eq!(action, PredictionAction::DisplayChar('a'));
        assert_eq!(engine.display_ahead, 1);

        let action = engine.on_keystroke("\x7f");
        assert_eq!(action, PredictionAction::DisplayBackspace);
        assert_eq!(engine.display_ahead, 0);

        // Server echoes "a" then BS — both predictions should be confirmed.
        let mut out = Vec::new();
        engine.process_server_output(b"a\x08", &mut out);
        assert_eq!(engine.cursor_col, 0, "cursor should return to 0 after a+BS");

        let all_confirmed = engine.epochs.iter().all(|e| {
            e.predictions
                .iter()
                .all(|p| p.state == PredictionState::Confirmed)
        });
        assert!(
            all_confirmed,
            "backspace prediction should be confirmed after server BS"
        );
    }

    #[test]
    fn incomplete_csi_should_pass_through_raw_bytes() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // ESC[ with no command byte — incomplete, should pass through bytes.
        engine.process_server_output(b"\x1b[", &mut out);
        assert_eq!(out, b"\x1b[");
    }

    #[test]
    fn csi_cursor_movement_during_pending_predictions_should_rollback() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        assert_eq!(engine.display_ahead, 1);

        let mut out = Vec::new();
        // Server sends CSI cursor forward 5 while a prediction is pending.
        engine.process_server_output(b"\x1b[5C", &mut out);

        // Rollback (SGR reset + cursor-left 1 + clear) is emitted BEFORE the CSI sequence.
        assert_eq!(out, b"\x1b[0m\x1b[1D\x1b[K\x1b[5C");
        assert_eq!(engine.display_ahead, 0);
        assert!(
            engine.epochs.is_empty(),
            "epochs should be cleared after CSI-triggered rollback"
        );
    }

    #[test]
    fn csi_sgr_during_pending_predictions_should_not_rollback() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        assert_eq!(engine.display_ahead, 1);

        let mut out = Vec::new();
        // Server sends an SGR sequence (bold red) — not a cursor-movement command.
        engine.process_server_output(b"\x1b[1;31m", &mut out);
        assert_eq!(out, b"\x1b[1;31m");
        assert_eq!(engine.display_ahead, 1, "SGR should not trigger rollback");

        // Now confirm the prediction with 'a' — overwrite sequence emitted.
        out.clear();
        engine.process_server_output(b"a", &mut out);
        assert_eq!(out, b"\x1b[1D\x1b[24ma");
        assert_eq!(engine.display_ahead, 0);
    }

    #[test]
    fn expired_multiple_predictions_should_emit_rollback() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        engine.on_keystroke("b");
        engine.on_keystroke("c");
        assert_eq!(engine.display_ahead, 3);

        // Backdate all predictions past PREDICTION_EXPIRY.
        for epoch in &mut engine.epochs {
            for pred in &mut epoch.predictions {
                pred.sent_at = Instant::now() - PREDICTION_EXPIRY - Duration::from_millis(100);
            }
        }

        // Process server output to trigger expire check.
        let mut out = Vec::new();
        engine.process_server_output(b"x", &mut out);

        // Expiry triggers rollback (SGR reset + cursor-left 3 + clear) then reset.
        // 'x' passes through since predictions were cleared.
        assert_eq!(out, b"\x1b[0m\x1b[3D\x1b[Kx");
        assert!(
            engine.epochs.is_empty(),
            "epochs should be empty after expiry-triggered reset"
        );
        assert_eq!(engine.display_ahead, 0);
    }

    #[test]
    fn overwrite_sequence_for_display_ahead_one() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        assert_eq!(engine.display_ahead, 1);

        let mut out = Vec::new();
        engine.process_server_output(b"a", &mut out);
        // display_ahead=1: cursor back 1, underline off, 'a', no cursor-forward (remaining=0).
        assert_eq!(out, b"\x1b[1D\x1b[24ma");
        assert_eq!(engine.display_ahead, 0);
    }

    #[test]
    fn overwrite_sequence_for_display_ahead_greater_than_one() {
        let mut engine = engine_on();
        engine.on_keystroke("a");
        engine.on_keystroke("b");
        engine.on_keystroke("c");
        assert_eq!(engine.display_ahead, 3);

        let mut out = Vec::new();
        // Confirm only 'a' — display_ahead=3: cursor back 3, underline off, 'a', cursor forward 2.
        engine.process_server_output(b"a", &mut out);
        assert_eq!(out, b"\x1b[3D\x1b[24ma\x1b[2C");
        assert_eq!(engine.display_ahead, 2);
    }

    #[test]
    fn alt_screen_enter_should_suppress_predictions() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // Enter alternate screen via DEC private mode 1049.
        engine.process_server_output(b"\x1b[?1049h", &mut out);
        assert!(
            !engine.should_display(),
            "should_display() should return false while in alternate screen"
        );
        // Keystroke while in alternate screen should return None.
        let action = engine.on_keystroke("a");
        assert_eq!(action, PredictionAction::None);
    }

    #[test]
    fn alt_screen_leave_should_resume_predictions() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        // Enter then leave alternate screen.
        engine.process_server_output(b"\x1b[?1049h", &mut out);
        assert!(!engine.should_display());
        engine.process_server_output(b"\x1b[?1049l", &mut out);
        assert!(
            engine.should_display(),
            "should_display() should return true after leaving alternate screen"
        );
        // Keystroke after leaving alternate screen should predict again.
        let action = engine.on_keystroke("a");
        assert_eq!(action, PredictionAction::DisplayChar('a'));
    }

    #[test]
    fn alt_screen_enter_with_pending_should_rollback() {
        let mut engine = engine_on();
        // Build up display_ahead = 2.
        engine.on_keystroke("a");
        engine.on_keystroke("b");
        assert_eq!(engine.display_ahead, 2);

        let mut out = Vec::new();
        // Alternate screen enter should rollback BEFORE writing the CSI.
        engine.process_server_output(b"\x1b[?1049h", &mut out);

        // Expected: rollback (SGR reset + cursor-left 2 + clear) then the CSI sequence.
        assert_eq!(out, b"\x1b[0m\x1b[2D\x1b[K\x1b[?1049h");
        assert_eq!(engine.display_ahead, 0);
        assert!(engine.in_alternate_screen);
    }

    #[test]
    fn alt_screen_mode_47_should_be_detected() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"\x1b[?47h", &mut out);
        assert!(engine.in_alternate_screen);
        assert!(
            !engine.should_display(),
            "DEC mode 47 should activate alternate screen detection"
        );
    }

    #[test]
    fn alt_screen_mode_1047_should_be_detected() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"\x1b[?1047h", &mut out);
        assert!(engine.in_alternate_screen);
        assert!(
            !engine.should_display(),
            "DEC mode 1047 should activate alternate screen detection"
        );
    }

    #[test]
    fn alt_screen_leave_mode_47_should_resume_predictions() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"\x1b[?47h", &mut out);
        assert!(engine.in_alternate_screen);
        out.clear();
        engine.process_server_output(b"\x1b[?47l", &mut out);
        assert!(!engine.in_alternate_screen);
        assert!(engine.should_display());
    }

    #[test]
    fn alt_screen_leave_mode_1047_should_resume_predictions() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"\x1b[?1047h", &mut out);
        assert!(engine.in_alternate_screen);
        out.clear();
        engine.process_server_output(b"\x1b[?1047l", &mut out);
        assert!(!engine.in_alternate_screen);
        assert!(engine.should_display());
    }

    #[test]
    fn reset_should_not_clear_alternate_screen_flag() {
        let mut engine = engine_on();
        let mut out = Vec::new();
        engine.process_server_output(b"\x1b[?1049h", &mut out);
        assert!(!engine.should_display());
        // reset() clears epochs/display_ahead but should preserve alternate screen state.
        engine.reset();
        assert!(
            !engine.should_display(),
            "reset() should not clear in_alternate_screen flag"
        );
        assert!(engine.in_alternate_screen);
    }

    #[test]
    fn alt_screen_enter_without_pending_should_set_flag_only() {
        let mut engine = engine_on();
        // No predictions — display_ahead is 0.
        assert_eq!(engine.display_ahead, 0);

        let mut out = Vec::new();
        engine.process_server_output(b"\x1b[?1049h", &mut out);

        // Output should contain only the CSI sequence, no rollback prefix.
        assert_eq!(out, b"\x1b[?1049h");
        assert!(engine.in_alternate_screen);
    }
}
