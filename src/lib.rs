#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

use std::process::{ExitCode, Termination};

use clap::error::ErrorKind;
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
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
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
                        Format::Shell => eprintln!("{x}"),
                        Format::Json => println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "type": "error",
                                "msg": x.to_string(),
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
