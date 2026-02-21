use std::borrow::Cow;
use std::time::Duration;

use console::{style, Term};
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
