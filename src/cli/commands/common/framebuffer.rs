//! Framebuffer-based terminal renderer with predictive echo overlay.
//!
//! Owns a vt100 parser (server byte interpretation), a ratatui terminal
//! (diff-based rendering), and a prediction overlay. Server output and
//! user keystrokes both flow through this struct, which renders a composed
//! frame (server screen + prediction overlay) to the local terminal.

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use log::trace;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier};

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

/// Framebuffer-based terminal renderer.
///
/// Shared between input and output tasks via `Arc<Mutex<>>`. The input
/// side calls [`on_keystroke`](Self::on_keystroke) to add predictions and
/// get encoded bytes. The output side calls
/// [`process_server_output`](Self::process_server_output) to update the
/// virtual screen and confirm predictions. Both methods trigger a render.
pub struct TerminalFramebuffer {
    vt_parser: vt100::Parser,
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    overlay: PredictionOverlay,
}

impl TerminalFramebuffer {
    /// Create a new framebuffer with the given terminal dimensions.
    ///
    /// Does NOT enter raw mode or alternate screen — the caller handles
    /// raw mode via crossterm. Uses `Viewport::Fixed` to render to the
    /// full terminal area without entering alternate screen.
    pub fn new(rows: u16, cols: u16, predict_mode: PredictMode) -> io::Result<Self> {
        let vt_parser = vt100::Parser::new(rows, cols, 0);
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::with_options(
            backend,
            ratatui::TerminalOptions {
                viewport: ratatui::Viewport::Fixed(Rect::new(0, 0, cols, rows)),
            },
        )?;

        Ok(Self {
            vt_parser,
            terminal,
            overlay: PredictionOverlay::new(predict_mode),
        })
    }

    /// Feed server output bytes through the sanitizer → vt100 → confirm → render pipeline.
    pub fn process_server_output(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut sanitized = Vec::with_capacity(bytes.len());
        TerminalSanitizer::CONPTY.filter(bytes, &mut sanitized);

        self.vt_parser.process(&sanitized);
        self.overlay.confirm_predictions(self.vt_parser.screen());
        self.render()
    }

    /// Record a user keystroke: add prediction, render, return encoded bytes
    /// to send to the server.
    pub fn on_keystroke(&mut self, event: &KeyEvent) -> Option<String> {
        let encoded = encode_key(event)?;
        self.overlay.on_input(&encoded, self.vt_parser.screen());
        // Render error is non-fatal: don't lose the keystroke over a display glitch.
        let _ = self.render();
        Some(encoded)
    }

    /// Handle terminal resize.
    pub fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        self.vt_parser.screen_mut().set_size(rows, cols);
        self.terminal.resize(Rect::new(0, 0, cols, rows))?;
        self.render()
    }

    /// Compose vt100 screen + prediction overlay → ratatui diff render.
    fn render(&mut self) -> io::Result<()> {
        let screen = self.vt_parser.screen();
        let overlay_cells = &self.overlay.cells;
        let should_display = self.overlay.should_display();
        let should_underline = self.overlay.should_underline();

        let (cursor_row, cursor_col) = screen.cursor_position();
        let cursor_col_final = if should_display {
            cursor_col.saturating_add(overlay_cells.len() as u16)
        } else {
            cursor_col
        };

        self.terminal.draw(|frame| {
            let area = frame.area();
            let buf = frame.buffer_mut();

            render_vt100_screen(screen, area, buf);

            if should_display {
                render_predictions(overlay_cells, should_underline, buf);
            }

            frame.set_cursor_position(Position::new(cursor_col_final, cursor_row));
        })?;

        Ok(())
    }

    /// Restore terminal state on shutdown. Writes SGR reset and mode
    /// disable sequences.
    pub fn shutdown(self) -> io::Result<()> {
        drop(self.terminal);

        let reset = TerminalSanitizer::CONPTY.reset_sequence();
        let stdout = io::stdout();
        let mut out = stdout.lock();
        {
            use io::Write;
            let _ = out.write_all(b"\x1b[0m");
            if !reset.is_empty() {
                let _ = out.write_all(&reset);
            }
            let _ = out.flush();
        }
        Ok(())
    }
}

/// Copy vt100 screen cells into the ratatui buffer.
fn render_vt100_screen(screen: &vt100::Screen, area: Rect, buf: &mut Buffer) {
    let (rows, cols) = screen.size();
    let height = area.height.min(rows);
    let width = area.width.min(cols);

    for row in 0..height {
        for col in 0..width {
            let Some(vt_cell) = screen.cell(row, col) else {
                continue;
            };
            if vt_cell.is_wide_continuation() {
                continue;
            }
            let Some(buf_cell) = buf.cell_mut(Position::new(area.x + col, area.y + row)) else {
                continue;
            };
            let contents = vt_cell.contents();
            if contents.is_empty() {
                buf_cell.set_symbol(" ");
            } else {
                buf_cell.set_symbol(contents);
            }
            buf_cell.fg = convert_color(vt_cell.fgcolor());
            buf_cell.bg = convert_color(vt_cell.bgcolor());
            buf_cell.modifier = convert_modifier(vt_cell);
        }
    }
}

/// Overlay predicted characters onto the ratatui buffer.
fn render_predictions(cells: &[PredictedCell], underline: bool, buf: &mut Buffer) {
    let modifier = if underline {
        Modifier::UNDERLINED
    } else {
        Modifier::empty()
    };

    for pred in cells {
        if let Some(buf_cell) = buf.cell_mut(Position::new(pred.col, pred.row)) {
            buf_cell.set_char(pred.ch);
            buf_cell.modifier = modifier;
        }
    }
}

/// Convert a vt100 color to a ratatui color.
fn convert_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Convert vt100 cell attributes to a ratatui modifier bitfield.
fn convert_modifier(cell: &vt100::Cell) -> Modifier {
    let mut m = Modifier::empty();
    if cell.bold() {
        m |= Modifier::BOLD;
    }
    if cell.italic() {
        m |= Modifier::ITALIC;
    }
    if cell.underline() {
        m |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        m |= Modifier::REVERSED;
    }
    if cell.dim() {
        m |= Modifier::DIM;
    }
    m
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

    #[test]
    fn convert_color_default_should_map_to_reset() {
        assert_eq!(convert_color(vt100::Color::Default), Color::Reset);
    }

    #[test]
    fn convert_color_idx_should_map_to_indexed() {
        assert_eq!(convert_color(vt100::Color::Idx(42)), Color::Indexed(42));
    }

    #[test]
    fn convert_color_rgb_should_map_to_rgb() {
        assert_eq!(
            convert_color(vt100::Color::Rgb(10, 20, 30)),
            Color::Rgb(10, 20, 30)
        );
    }

    #[test]
    fn convert_modifier_bold_cell() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[1mX"); // bold + char
        let cell = parser.screen().cell(0, 0).unwrap();
        let m = convert_modifier(cell);
        assert!(m.contains(Modifier::BOLD));
    }

    #[test]
    fn convert_modifier_italic_cell() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[3mX"); // italic
        let cell = parser.screen().cell(0, 0).unwrap();
        let m = convert_modifier(cell);
        assert!(m.contains(Modifier::ITALIC));
    }

    #[test]
    fn convert_modifier_underline_cell() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[4mX"); // underline
        let cell = parser.screen().cell(0, 0).unwrap();
        let m = convert_modifier(cell);
        assert!(m.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn convert_modifier_plain_cell_should_be_empty() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"X");
        let cell = parser.screen().cell(0, 0).unwrap();
        let m = convert_modifier(cell);
        assert!(m.is_empty());
    }

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
    fn prediction_placement_at_cursor() {
        let mut o = overlay_on_confirmed();
        let parser = parser_80x24();
        o.on_input("a", parser.screen());
        assert_eq!(o.cells.len(), 1);
        assert_eq!(o.cells[0].ch, 'a');
        assert_eq!(o.cells[0].row, 0);
        assert_eq!(o.cells[0].col, 0);
    }

    #[test]
    fn prediction_offset_for_multiple_chars() {
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
    fn render_screen_should_copy_text_content() {
        let mut parser = parser_80x24();
        parser.process(b"Hello");

        let rect = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(rect);
        render_vt100_screen(parser.screen(), rect, &mut buf);

        let cell_h = buf.cell(Position::new(0, 0)).unwrap();
        assert_eq!(cell_h.symbol(), "H");
        let cell_e = buf.cell(Position::new(1, 0)).unwrap();
        assert_eq!(cell_e.symbol(), "e");
    }

    #[test]
    fn render_screen_should_copy_colors() {
        let mut parser = parser_80x24();
        // Red foreground: ESC[31mX
        parser.process(b"\x1b[31mX");

        let rect = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(rect);
        render_vt100_screen(parser.screen(), rect, &mut buf);

        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert_eq!(cell.fg, Color::Indexed(1)); // Red = index 1
    }

    #[test]
    fn render_predictions_should_overlay_chars() {
        let rect = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(rect);

        let cells = vec![PredictedCell {
            row: 0,
            col: 5,
            ch: 'X',
            epoch: 0,
            sent_at: Instant::now(),
        }];
        render_predictions(&cells, false, &mut buf);

        let cell = buf.cell(Position::new(5, 0)).unwrap();
        assert_eq!(cell.symbol(), "X");
        assert!(cell.modifier.is_empty());
    }

    #[test]
    fn render_predictions_underline_should_set_modifier() {
        let rect = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(rect);

        let cells = vec![PredictedCell {
            row: 0,
            col: 0,
            ch: 'A',
            epoch: 0,
            sent_at: Instant::now(),
        }];
        render_predictions(&cells, true, &mut buf);

        let cell = buf.cell(Position::new(0, 0)).unwrap();
        assert!(cell.modifier.contains(Modifier::UNDERLINED));
    }
}
