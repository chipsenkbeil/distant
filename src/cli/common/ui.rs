use std::borrow::Cow;
use std::time::Duration;

use console::{Term, style};
use indicatif::{ProgressBar, ProgressStyle};

/// Terminal UI abstraction for all visual feedback.
///
/// All status/feedback output goes to stderr. Stdout is reserved for data/JSON output.
/// Automatically detects whether stderr is a TTY and adjusts output accordingly:
/// - Interactive (TTY): spinners, colors, unicode symbols
/// - Non-interactive (piped): plain text prefixes, no ANSI codes
pub struct Ui {
    term: Term,
    interactive: bool,
}

impl Ui {
    /// Create a new UI that writes to stderr.
    pub fn new() -> Self {
        let term = Term::stderr();
        let interactive = term.is_term();
        Self { term, interactive }
    }

    /// Start a spinner with the given message.
    ///
    /// In interactive mode, shows an animated spinner. In non-interactive mode,
    /// prints the message once and returns a no-op spinner.
    pub fn spinner(&self, msg: &str) -> Spinner {
        if self.interactive {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", " "])
                    .template("{spinner} {msg}")
                    .expect("invalid spinner template"),
            );
            pb.set_message(msg.to_string());
            pb.enable_steady_tick(Duration::from_millis(80));
            Spinner {
                pb: Some(pb),
                interactive: true,
                term: self.term.clone(),
            }
        } else {
            let _ = self.term.write_line(msg);
            Spinner {
                pb: None,
                interactive: false,
                term: self.term.clone(),
            }
        }
    }

    /// Print a success message: "✓ msg" (green) or "msg" (plain).
    pub fn success(&self, msg: &str) {
        if self.interactive {
            let _ = self
                .term
                .write_line(&format!("{} {}", style("✓").green(), msg));
        } else {
            let _ = self.term.write_line(msg);
        }
    }

    /// Print a warning message: "⚠ msg" (yellow) or "warning: msg" (plain).
    pub fn warning(&self, msg: &str) {
        if self.interactive {
            let _ = self
                .term
                .write_line(&format!("{} {}", style("⚠").yellow(), msg));
        } else {
            let _ = self.term.write_line(&format!("warning: {msg}"));
        }
    }

    /// Print an error message: "✗ msg" (red) or "error: msg" (plain).
    pub fn error(&self, msg: &str) {
        if self.interactive {
            let _ = self
                .term
                .write_line(&format!("{} {}", style("✗").red(), msg));
        } else {
            let _ = self.term.write_line(&format!("error: {msg}"));
        }
    }

    /// Print an error message with a suggestion underneath.
    pub fn error_with_suggestion(&self, msg: &str, suggestion: &str) {
        if self.interactive {
            let _ = self
                .term
                .write_line(&format!("{} {}", style("✗").red(), msg));
            let _ = self
                .term
                .write_line(&format!("  {}", style(suggestion).dim()));
        } else {
            let _ = self.term.write_line(&format!("error: {msg}"));
            let _ = self.term.write_line(&format!("  {suggestion}"));
        }
    }

    /// Print a header/title: bold or plain.
    pub fn header(&self, msg: &str) {
        if self.interactive {
            let _ = self.term.write_line(&format!("{}", style(msg).bold()));
        } else {
            let _ = self.term.write_line(msg);
        }
    }

    /// Print dim/secondary text.
    pub fn dim(&self, msg: &str) {
        if self.interactive {
            let _ = self.term.write_line(&format!("{}", style(msg).dim()));
        } else {
            let _ = self.term.write_line(msg);
        }
    }

    /// Print a "Label: value" status line with colored value.
    pub fn status(&self, label: &str, value: &str, color: StatusColor) {
        if self.interactive {
            let styled_value = match color {
                StatusColor::Green => style(value).green(),
                StatusColor::Red => style(value).red(),
                StatusColor::Yellow => style(value).yellow(),
            };
            let _ = self
                .term
                .write_line(&format!("{}: {}", style(label).bold(), styled_value));
        } else {
            let _ = self.term.write_line(&format!("{label}: {value}"));
        }
    }

    /// Write a raw line to the terminal (stderr).
    pub fn write_line(&self, msg: &str) {
        let _ = self.term.write_line(msg);
    }

    /// Whether the terminal is interactive (TTY).
    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// Get a reference to the underlying terminal.
    pub fn term(&self) -> &Term {
        &self.term
    }
}

/// Color for status output.
pub enum StatusColor {
    Green,
    Red,
    Yellow,
}

/// A spinner that wraps `indicatif::ProgressBar` in interactive mode,
/// or is a no-op in non-interactive mode.
pub struct Spinner {
    pb: Option<ProgressBar>,
    interactive: bool,
    term: Term,
}

impl Spinner {
    /// Update the spinner message.
    pub fn set_message(&self, msg: impl Into<Cow<'static, str>>) {
        if let Some(ref pb) = self.pb {
            pb.set_message(msg);
        }
    }

    /// Stop the spinner with a success message: "✓ msg" (green).
    pub fn done(self, msg: &str) {
        if let Some(pb) = self.pb {
            pb.finish_and_clear();
            if self.interactive {
                let _ = self
                    .term
                    .write_line(&format!("{} {}", style("✓").green(), msg));
            } else {
                let _ = self.term.write_line(msg);
            }
        }
    }

    /// Get the underlying `ProgressBar`, if interactive.
    ///
    /// This is useful for coordinating I/O with the spinner via `ProgressBar::suspend()`.
    pub fn progress_bar(&self) -> Option<ProgressBar> {
        self.pb.clone()
    }

    /// Stop the spinner with a failure message: "✗ msg" (red).
    pub fn fail(self, msg: &str) {
        if let Some(pb) = self.pb {
            pb.finish_and_clear();
            if self.interactive {
                let _ = self
                    .term
                    .write_line(&format!("{} {}", style("✗").red(), msg));
            } else {
                let _ = self.term.write_line(&format!("error: {msg}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Smoke tests for `Ui`, `Spinner`, and `StatusColor`. These tests verify
    //! that methods do not panic; they do not assert on terminal output content
    //! because `Ui` writes directly to a terminal handle, making output capture
    //! impractical in a test harness.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // Ui::new — construction
    // -------------------------------------------------------
    #[test]
    fn ui_new_does_not_panic() {
        let _ui = Ui::new();
    }

    // -------------------------------------------------------
    // Ui::is_interactive — reflects terminal detection
    // -------------------------------------------------------
    #[test]
    fn ui_is_interactive_returns_bool() {
        let ui = Ui::new();
        // In CI / test runner, stderr is typically NOT a TTY
        // We just verify it doesn't panic and returns a bool
        let _result = ui.is_interactive();
    }

    // -------------------------------------------------------
    // Ui::term — gives access to underlying terminal
    // -------------------------------------------------------
    #[test]
    fn ui_term_returns_term_reference() {
        let ui = Ui::new();
        let _term = ui.term();
    }

    // -------------------------------------------------------
    // Ui methods — verify they don't panic
    // -------------------------------------------------------
    #[test]
    fn ui_success_does_not_panic() {
        let ui = Ui::new();
        ui.success("test success message");
    }

    #[test]
    fn ui_warning_does_not_panic() {
        let ui = Ui::new();
        ui.warning("test warning message");
    }

    #[test]
    fn ui_error_does_not_panic() {
        let ui = Ui::new();
        ui.error("test error message");
    }

    #[test]
    fn ui_error_with_suggestion_does_not_panic() {
        let ui = Ui::new();
        ui.error_with_suggestion("error msg", "try this instead");
    }

    #[test]
    fn ui_header_does_not_panic() {
        let ui = Ui::new();
        ui.header("test header");
    }

    #[test]
    fn ui_dim_does_not_panic() {
        let ui = Ui::new();
        ui.dim("dimmed text");
    }

    #[test]
    fn ui_status_does_not_panic() {
        let ui = Ui::new();
        ui.status("Status", "connected", StatusColor::Green);
        ui.status("Status", "disconnected", StatusColor::Red);
        ui.status("Status", "pending", StatusColor::Yellow);
    }

    #[test]
    fn ui_write_line_does_not_panic() {
        let ui = Ui::new();
        ui.write_line("raw output");
    }

    // -------------------------------------------------------
    // Spinner — non-interactive mode
    // -------------------------------------------------------
    #[test]
    fn spinner_in_non_interactive_mode() {
        let ui = Ui::new();
        // When not interactive, spinner methods are no-ops
        let spinner = ui.spinner("loading...");
        spinner.set_message("still loading...");
        // done/fail won't have a pb in non-interactive mode
        // but should not panic regardless
    }

    #[test]
    fn spinner_done_does_not_panic() {
        let ui = Ui::new();
        let spinner = ui.spinner("loading...");
        spinner.done("done!");
    }

    #[test]
    fn spinner_fail_does_not_panic() {
        let ui = Ui::new();
        let spinner = ui.spinner("loading...");
        spinner.fail("failed!");
    }

    #[test]
    fn spinner_progress_bar_reflects_interactivity() {
        let ui = Ui::new();
        let spinner = ui.spinner("loading...");
        // In non-interactive mode (typical for tests), pb should be None
        if !ui.is_interactive() {
            assert!(spinner.progress_bar().is_none());
        }
    }

    // -------------------------------------------------------
    // StatusColor — enum variant existence
    // -------------------------------------------------------
    #[test]
    fn status_color_variants_exist() {
        let _green = StatusColor::Green;
        let _red = StatusColor::Red;
        let _yellow = StatusColor::Yellow;
    }
}
