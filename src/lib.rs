#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

use std::process::{ExitCode, Termination};

use derive_more::{Display, Error, From};

mod cli;
mod constants;
mod options;

#[cfg(windows)]
pub mod win_service;

pub use cli::Cli;
pub use options::Options;

/// Wrapper around a [`CliResult`] that provides [`Termination`] support
pub struct MainResult(CliResult);

impl MainResult {
    pub const OK: MainResult = MainResult(Ok(()));
}

impl From<CliResult> for MainResult {
    fn from(res: CliResult) -> Self {
        Self(res)
    }
}

impl From<anyhow::Error> for MainResult {
    fn from(x: anyhow::Error) -> Self {
        Self(Err(CliError::Error(x)))
    }
}

impl From<anyhow::Result<()>> for MainResult {
    fn from(res: anyhow::Result<()>) -> Self {
        Self(res.map_err(CliError::Error))
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
        match self.0 {
            Ok(_) => ExitCode::SUCCESS,
            Err(x) => match x {
                CliError::Exit(code) => ExitCode::from(code),
                CliError::Error(x) => {
                    eprintln!("{x:?}");
                    ::log::error!("{x:?}");
                    ::log::logger().flush();
                    ExitCode::FAILURE
                }
            },
        }
    }
}
