use crate::{ExitCode, ExitCodeError};
use derive_more::{Display, Error, From};

pub type CliResult<T> = Result<T, CliError>;

/// Error encountered during operating the CLI
#[derive(Debug, Display, Error, From)]
pub enum CliError {
    Io(std::io::Error),
    ExitCode(#[error(not(source))] ExitCode),

    #[display(fmt = "Need to pick a connection as there are multiple choices")]
    NeedToPickConnection,

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
            Self::Io(x) => x.to_exit_code(),
            Self::ExitCode(x) => *x,
            Self::NeedToPickConnection => ExitCode::Unavailable,
            Self::NoConnection => ExitCode::Unavailable,
        }
    }
}
