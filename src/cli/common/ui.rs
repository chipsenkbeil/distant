use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use console::{Term, style};

/// Terminal UI abstraction for all visual feedback.
///
/// All status/feedback output goes to stderr. Stdout is reserved for data/JSON output.
/// Automatically detects whether stderr is a TTY and adjusts output accordingly:
/// - Interactive (TTY): spinners, colors, unicode symbols
/// - Non-interactive (piped): plain text prefixes, no ANSI codes
pub struct Ui {
    term: Term,
    interactive: bool,
    quiet: bool,
}

impl Ui {
    /// Create a new UI that writes to stderr.
    ///
    /// When `quiet` is true, all informational output (spinners, success/error messages,
    /// headers, etc.) is suppressed. This is useful for scripting and pipeline use.
    pub fn new(quiet: bool) -> Self {
        let term = Term::stderr();
        let interactive = term.is_term();
        Self {
            term,
            interactive,
            quiet,
        }
    }

    /// Start a spinner with the given message.
    ///
    /// In interactive mode, shows an animated spinner. In non-interactive mode,
    /// prints the message once and returns a no-op spinner. In quiet mode,
    /// returns an inert spinner with no output.
    pub fn spinner(&self, msg: &str) -> Spinner {
        if self.quiet {
            return Spinner {
                inner: None,
                interactive: false,
                term: self.term.clone(),
            };
        }
        if self.interactive {
            let inner = SpinnerInner::start(msg, self.term.clone());
            Spinner {
                inner: Some(inner),
                interactive: true,
                term: self.term.clone(),
            }
        } else {
            let _ = self.term.write_line(msg);
            Spinner {
                inner: None,
                interactive: false,
                term: self.term.clone(),
            }
        }
    }

    /// Print a success message: "✓ msg" (green) or "msg" (plain).
    pub fn success(&self, msg: &str) {
        if self.quiet {
            return;
        }
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
        if self.quiet {
            return;
        }
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
        if self.quiet {
            return;
        }
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
        if self.quiet {
            return;
        }
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
        if self.quiet {
            return;
        }
        if self.interactive {
            let _ = self.term.write_line(&format!("{}", style(msg).bold()));
        } else {
            let _ = self.term.write_line(msg);
        }
    }

    /// Print dim/secondary text.
    pub fn dim(&self, msg: &str) {
        if self.quiet {
            return;
        }
        if self.interactive {
            let _ = self.term.write_line(&format!("{}", style(msg).dim()));
        } else {
            let _ = self.term.write_line(msg);
        }
    }

    /// Print a "Label: value" status line with colored value.
    pub fn status(&self, label: &str, value: &str, color: StatusColor) {
        if self.quiet {
            return;
        }
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
        if self.quiet {
            return;
        }
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

/// Braille spinner frames (same sequence as the former indicatif config).
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Internal state shared between the spinner thread and the `Spinner` handle.
struct SpinnerInner {
    message: Arc<Mutex<String>>,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl SpinnerInner {
    /// Starts the background animation thread.
    fn start(msg: &str, term: Term) -> Self {
        let message = Arc::new(Mutex::new(msg.to_string()));
        let running = Arc::new(AtomicBool::new(true));
        let paused = Arc::new(AtomicBool::new(false));

        let msg_clone = Arc::clone(&message);
        let running_clone = Arc::clone(&running);
        let paused_clone = Arc::clone(&paused);

        let thread = std::thread::spawn(move || {
            let mut frame = 0usize;
            while running_clone.load(Ordering::Relaxed) {
                if !paused_clone.load(Ordering::Relaxed) {
                    let symbol = SPINNER_FRAMES[frame % SPINNER_FRAMES.len()];
                    let msg = msg_clone.lock().unwrap().clone();
                    let _ = term.clear_line();
                    let _ = term.write_str(&format!("{symbol} {msg}"));
                    frame += 1;
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            let _ = term.clear_line();
        });

        Self {
            message,
            running,
            paused,
            thread: Some(thread),
        }
    }

    /// Stops the animation thread.
    fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for SpinnerInner {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A spinner that shows an animated braille pattern in interactive mode,
/// or is a no-op in non-interactive mode.
pub struct Spinner {
    inner: Option<SpinnerInner>,
    interactive: bool,
    term: Term,
}

impl Spinner {
    /// Update the spinner message.
    pub fn set_message(&self, msg: impl Into<Cow<'static, str>>) {
        if let Some(ref inner) = self.inner {
            *inner.message.lock().unwrap() = msg.into().into_owned();
        }
    }

    /// Stop the spinner with a success message: "✓ msg" (green).
    pub fn done(mut self, msg: &str) {
        if let Some(ref mut inner) = self.inner {
            inner.stop();
            if self.interactive {
                let _ = self
                    .term
                    .write_line(&format!("{} {}", style("✓").green(), msg));
            } else {
                let _ = self.term.write_line(msg);
            }
        }
    }

    /// Pause the spinner, run the closure, and resume.
    ///
    /// This prevents visual conflicts when prompting for user input on stderr
    /// while the spinner is animating.
    pub fn suspend<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        if let Some(ref inner) = self.inner {
            inner.paused.store(true, Ordering::Relaxed);
            // Small sleep to let the animation thread finish its current frame
            std::thread::sleep(Duration::from_millis(100));
            let _ = self.term.clear_line();
            let result = f();
            inner.paused.store(false, Ordering::Relaxed);
            result
        } else {
            f()
        }
    }

    /// Returns a suspend handle that can be shared with other code (e.g. auth handlers)
    /// to pause the spinner during prompts. Returns `None` in non-interactive mode.
    pub fn suspend_handle(&self) -> Option<SuspendHandle> {
        self.inner.as_ref().map(|inner| SuspendHandle {
            paused: Arc::clone(&inner.paused),
            term: self.term.clone(),
        })
    }

    /// Stop the spinner with a failure message: "✗ msg" (red).
    pub fn fail(mut self, msg: &str) {
        if let Some(ref mut inner) = self.inner {
            inner.stop();
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

/// Shared suspend handle that can pause a spinner while prompting for input.
///
/// Wraps the spinner's `paused` flag and terminal handle, allowing the
/// `PromptAuthHandler` to temporarily stop spinner animation during user prompts.
#[derive(Clone)]
pub struct SuspendHandle {
    paused: Arc<AtomicBool>,
    term: Term,
}

impl SuspendHandle {
    /// Pause the spinner, run the closure, and resume.
    pub fn suspend<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        self.paused.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(100));
        let _ = self.term.clear_line();
        let result = f();
        self.paused.store(false, Ordering::Relaxed);
        result
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
        let _ui = Ui::new(false);
    }

    // -------------------------------------------------------
    // Ui::is_interactive — reflects terminal detection
    // -------------------------------------------------------
    #[test]
    fn ui_is_interactive_returns_bool() {
        let ui = Ui::new(false);
        // In CI / test runner, stderr is typically NOT a TTY
        // We just verify it doesn't panic and returns a bool
        let _result = ui.is_interactive();
    }

    // -------------------------------------------------------
    // Ui::term — gives access to underlying terminal
    // -------------------------------------------------------
    #[test]
    fn ui_term_returns_term_reference() {
        let ui = Ui::new(false);
        let _term = ui.term();
    }

    // -------------------------------------------------------
    // Ui methods — verify they don't panic
    // -------------------------------------------------------
    #[test]
    fn ui_success_does_not_panic() {
        let ui = Ui::new(false);
        ui.success("test success message");
    }

    #[test]
    fn ui_warning_does_not_panic() {
        let ui = Ui::new(false);
        ui.warning("test warning message");
    }

    #[test]
    fn ui_error_does_not_panic() {
        let ui = Ui::new(false);
        ui.error("test error message");
    }

    #[test]
    fn ui_error_with_suggestion_does_not_panic() {
        let ui = Ui::new(false);
        ui.error_with_suggestion("error msg", "try this instead");
    }

    #[test]
    fn ui_header_does_not_panic() {
        let ui = Ui::new(false);
        ui.header("test header");
    }

    #[test]
    fn ui_dim_does_not_panic() {
        let ui = Ui::new(false);
        ui.dim("dimmed text");
    }

    #[test]
    fn ui_status_does_not_panic() {
        let ui = Ui::new(false);
        ui.status("Status", "connected", StatusColor::Green);
        ui.status("Status", "disconnected", StatusColor::Red);
        ui.status("Status", "pending", StatusColor::Yellow);
    }

    #[test]
    fn ui_write_line_does_not_panic() {
        let ui = Ui::new(false);
        ui.write_line("raw output");
    }

    // -------------------------------------------------------
    // Spinner — non-interactive mode
    // -------------------------------------------------------
    #[test]
    fn spinner_in_non_interactive_mode() {
        let ui = Ui::new(false);
        // When not interactive, spinner methods are no-ops
        let spinner = ui.spinner("loading...");
        spinner.set_message("still loading...");
        // done/fail won't have an inner in non-interactive mode
        // but should not panic regardless
    }

    #[test]
    fn spinner_done_does_not_panic() {
        let ui = Ui::new(false);
        let spinner = ui.spinner("loading...");
        spinner.done("done!");
    }

    #[test]
    fn spinner_fail_does_not_panic() {
        let ui = Ui::new(false);
        let spinner = ui.spinner("loading...");
        spinner.fail("failed!");
    }

    #[test]
    fn spinner_suspend_runs_closure() {
        let ui = Ui::new(false);
        let spinner = ui.spinner("loading...");
        let result = spinner.suspend(|| 42);
        assert_eq!(result, 42);
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
