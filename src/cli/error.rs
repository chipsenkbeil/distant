use crate::{ExitCode, ExitCodeError};
use derive_more::{Display, Error, From};

pub type CliResult<T> = Result<T, CliError>;

/// Error encountered during operating the CLI
#[derive(Debug, Display, Error, From)]
pub enum CliError {
    /// Arguments provided to CLI are incorrect
    Usage(clap::Error),

    /// General purpose IO error
    Io(std::io::Error),

    /// No information exists, just an exit code
    ExitCode(#[error(not(source))] ExitCode),

    /// When there is more than one connection being managed
    #[display(fmt = "Need to pick a connection as there are multiple choices")]
    NeedToPickConnection,

    /// Whether there is no connection being managed
    #[display(fmt = "No active connection exists")]
    NoConnection,
}

impl From<i32> for CliError {
    fn from(code: i32) -> Self {
        Self::ExitCode(code.into())
    }
}

impl ExitCodeError for CliError {
    /// Returns true if error is just an exit code
    fn is_silent(&self) -> bool {
        matches!(self, Self::ExitCode(_))
    }

    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::Usage(_) => ExitCode::Usage,
            Self::Io(x) => x.to_exit_code(),
            Self::ExitCode(x) => *x,
            Self::NeedToPickConnection => ExitCode::Unavailable,
            Self::NoConnection => ExitCode::Unavailable,
        }
    }
}
