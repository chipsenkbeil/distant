#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

use std::process::{ExitCode, Termination};

use clap::error::ErrorKind;
use console::{style, Term};
use derive_more::{Display, Error, From};

mod cli;
mod constants;
mod options;

#[cfg(windows)]
pub mod win_service;

pub use cli::Cli;
pub use options::{Format, Options, OptionsError};

/// Wrapper around a [`CliResult`] that provides [`Termination`] support and [`Format`]ing.
pub struct MainResult {
    inner: CliResult,
    format: Format,
}

impl MainResult {
    pub const OK: MainResult = MainResult {
        inner: Ok(()),
        format: Format::Shell,
    };

    /// Creates a new result that performs general shell formatting.
    pub fn new(inner: CliResult) -> Self {
        Self {
            inner,
            format: Format::Shell,
        }
    }

    /// Converts to shell formatting for errors.
    pub fn shell(self) -> Self {
        Self {
            inner: self.inner,
            format: Format::Shell,
        }
    }

    /// Converts to a JSON formatting for errors.
    pub fn json(self) -> Self {
        Self {
            inner: self.inner,
            format: Format::Json,
        }
    }
}

impl From<CliResult> for MainResult {
    fn from(res: CliResult) -> Self {
        Self::new(res)
    }
}

impl From<OptionsError> for MainResult {
    fn from(x: OptionsError) -> Self {
        Self::new(match x {
            OptionsError::Config(x) => Err(CliError::Error(x)),
            OptionsError::Options(x) => match x.kind() {
                // --help and --version should not actually exit with an error and instead display
                // their related information while succeeding
                ErrorKind::DisplayHelp
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                | ErrorKind::DisplayVersion => {
                    // NOTE: We're causing a side effect here in constructing the main result,
                    //       but seems cleaner than returning an error with an exit code of 0
                    //       and a message to try to print. Plus, we leverage automatic color
                    //       handling in this approach.
                    let _ = x.print();
                    Ok(())
                }

                // Everything else is an actual error and should fail
                _ => Err(CliError::Error(anyhow::anyhow!(x))),
            },
        })
    }
}

impl From<anyhow::Error> for MainResult {
    fn from(x: anyhow::Error) -> Self {
        Self::new(Err(CliError::Error(x)))
    }
}

impl From<anyhow::Result<()>> for MainResult {
    fn from(res: anyhow::Result<()>) -> Self {
        Self::new(res.map_err(CliError::Error))
    }
}

pub type CliResult = Result<(), CliError>;

/// Represents an error associated with the CLI
#[derive(Debug, Display, Error, From)]
pub enum CliError {
    /// CLI should return a specific error code
    Exit(#[error(not(source))] u8),

    /// CLI encountered some unexpected error
    Error(#[error(not(source))] anyhow::Error),
}

impl CliError {
    /// Represents a generic failure with exit code = 1
    pub const FAILURE: CliError = CliError::Exit(1);
}

impl Termination for MainResult {
    fn report(self) -> ExitCode {
        match self.inner {
            Ok(_) => ExitCode::SUCCESS,
            Err(x) => match x {
                CliError::Exit(code) => ExitCode::from(code),
                CliError::Error(x) => {
                    match self.format {
                        Format::Shell => format_error_for_shell(&x),

                        Format::Json => println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "type": "error",
                                "msg": format!("{x:?}"),
                            }),)
                            .expect("Failed to format error to JSON")
                        ),
                    }

                    ::log::error!("{x:?}");
                    ::log::logger().flush();

                    ExitCode::FAILURE
                }
            },
        }
    }
}

/// Format an anyhow error for human-readable shell output.
///
/// Produces colored output with cause chain and contextual suggestions
/// when stderr is a TTY; plain text otherwise.
fn format_error_for_shell(err: &anyhow::Error) {
    let term = Term::stderr();
    let interactive = term.is_term();

    // Top-level error message
    let top_msg = format!("{err}");
    if interactive {
        let _ = term.write_line(&format!("{} {}", style("✗").red(), style(&top_msg).red()));
    } else {
        let _ = term.write_line(&format!("error: {top_msg}"));
    }

    // Cause chain (skip the first, which is the top-level message)
    let mut causes: Vec<String> = err.chain().skip(1).map(|e| format!("{e}")).collect();
    // Deduplicate adjacent causes that are identical
    causes.dedup();

    if !causes.is_empty() {
        for cause in &causes {
            if interactive {
                let _ = term.write_line(&format!(
                    "  {} {}",
                    style("caused by:").dim(),
                    style(cause).dim()
                ));
            } else {
                let _ = term.write_line(&format!("  caused by: {cause}"));
            }
        }
    }

    // Gather all text for suggestion matching
    let full_msg = {
        let mut parts = vec![top_msg.clone()];
        parts.extend(causes);
        parts.join(" ")
    };
    let lower = full_msg.to_lowercase();

    let suggestions = suggestions_for_error(&lower);
    if !suggestions.is_empty() {
        let _ = term.write_line("");
        if interactive {
            let _ = term.write_line(&format!("  {}:", style("Try").bold()));
        } else {
            let _ = term.write_line("  Try:");
        }
        for (cmd, desc) in &suggestions {
            if interactive {
                let _ =
                    term.write_line(&format!("    {}  {}", style(cmd).cyan(), style(desc).dim()));
            } else {
                let _ = term.write_line(&format!("    {cmd}  {desc}"));
            }
        }
    }
}

/// Return contextual suggestions based on error message patterns.
fn suggestions_for_error(msg: &str) -> Vec<(&'static str, &'static str)> {
    let mut suggestions = Vec::new();

    if msg.contains("connect") && msg.contains("manager")
        || msg.contains("no such file")
        || msg.contains("connection refused")
        || msg.contains("no unix socket")
        || msg.contains("no windows pipe")
    {
        suggestions.push(("distant manager listen --daemon", "Start the manager first"));
        suggestions.push(("distant status", "Check current status"));
    }

    if msg.contains("no active connections") {
        suggestions.push(("distant ssh user@host", "Connect via SSH"));
        suggestions.push((
            "distant connect ssh://user@host",
            "Connect to a remote server",
        ));
    }

    if msg.contains("authentication")
        || msg.contains("auth failed")
        || msg.contains("permission denied")
    {
        suggestions.push(("ssh-add -l", "Check loaded SSH keys"));
        suggestions.push(("ssh-add ~/.ssh/id_ed25519", "Add your SSH key to the agent"));
    }

    if msg.contains("multiple active connections") {
        suggestions.push(("distant status", "See available connections"));
        suggestions.push((
            "distant shell --connection ID",
            "Specify a connection directly",
        ));
    }

    suggestions
}
