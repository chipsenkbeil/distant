use derive_more::{Display, Error};
use std::error::Error;

/// Exit codes following https://www.freebsd.org/cgi/man.cgi?query=sysexits&sektion=3
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Hash)]
pub enum ExitCode {
    /// `EX_USAGE` (64) - command was used incorrectly
    Usage,

    /// `EX_DATAERR` (65) - input data was incorrect in some way
    DataErr,

    /// `EX_NOINPUT` (66) - input file did not exist or was not readable
    NoInput,

    /// `EX_NOUSER` (67) - user specified did not exist for remote login
    NoUser,

    /// `EX_NOHOST` (68) - host specified did not exist
    NoHost,

    /// `EX_UNAVAILABLE` (69) - service is unavailable (e.g. network error)
    Unavailable,

    /// `EX_SOFTWARE` (70) - internal software error has been detected (e.g. action failed)
    Software,

    /// `EX_OSERR` (71) - operating system error has been detected (e.g. fork failed)
    OsErr,

    /// `EX_IOERR` (74) - error occurred while doing I/O
    IoError,

    /// `EX_TEMPFAIL` (75) - temporary failure, indicating something that can be retried later
    TempFail,

    /// `EX_PROTOCOL` (76) - remote system returned something that was "not possible" during a protocol exchange
    Protocol,

    /// `EX_NOPERM` (77) - you did not have sufficient permission to perform the operation
    NoPermission,

    /// `EX_CONFIG` (78) - something was found in an unconfigured or misconfigured state
    Config,

    /// Custom exit code to pass back verbatim
    Custom(i32),
}

impl ExitCode {
    /// Convert into numeric exit code
    pub fn to_i32(self) -> i32 {
        match self {
            Self::Usage => 64,
            Self::DataErr => 65,
            Self::NoInput => 66,
            Self::NoUser => 67,
            Self::NoHost => 68,
            Self::Unavailable => 69,
            Self::Software => 70,
            Self::OsErr => 71,
            Self::IoError => 74,
            Self::TempFail => 75,
            Self::Protocol => 76,
            Self::NoPermission => 77,
            Self::Config => 78,
            Self::Custom(x) => x,
        }
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_USAGE` error code and `error`
    pub fn usage_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::Usage, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_DATAERR` error code and `error`
    pub fn data_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::DataErr, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_NOINPUT` error code and `error`
    pub fn no_input_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::NoInput, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_NOUSER` error code and `error`
    pub fn no_user_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::NoUser, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_NOHOST` error code and `error`
    pub fn no_host_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::NoHost, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_UNAVAILABLE` error code and `error`
    pub fn unavailable_error(
        error: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::Unavailable, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_SOFTWARE` error code and `error`
    pub fn software_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::Software, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_OSERR` error code and `error`
    pub fn os_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::OsErr, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_IOERR` error code and `error`
    pub fn io_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::IoError, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_TEMPFAIL` error code and `error`
    pub fn temp_fail_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::TempFail, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_PROTOCOL` error code and `error`
    pub fn protocol_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::Protocol, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_NOPERM` error code and `error`
    pub fn no_permission_error(
        error: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::NoPermission, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with `EX_CONFIG` error code and `error`
    pub fn config_error(error: impl Into<Box<dyn Error + Send + Sync>>) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::Config, error)
    }

    /// Create a new [`DescriptiveExitCodeError`] with custom error code and `error`
    pub fn custom_error(
        code: i32,
        error: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> WrappedExitCodeError {
        WrappedExitCodeError::new(Self::Custom(code), error)
    }
}

impl From<i32> for ExitCode {
    fn from(code: i32) -> Self {
        match code {
            64 => Self::Usage,
            65 => Self::DataErr,
            66 => Self::NoInput,
            67 => Self::NoUser,
            68 => Self::NoHost,
            69 => Self::Unavailable,
            70 => Self::Software,
            71 => Self::OsErr,
            74 => Self::IoError,
            75 => Self::TempFail,
            76 => Self::Protocol,
            77 => Self::NoPermission,
            78 => Self::Config,
            x => Self::Custom(x),
        }
    }
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> Self {
        code.to_i32()
    }
}

/// Represents an error that can be converted into an exit code
pub trait ExitCodeError: std::error::Error {
    fn to_exit_code(&self) -> ExitCode;

    /// Indicates if the error message associated with this exit code error
    /// should be printed, or if this is just used to reflect the exit code
    /// when the process exits
    fn is_silent(&self) -> bool {
        false
    }

    fn to_i32(&self) -> i32 {
        self.to_exit_code().to_i32()
    }
}

impl ExitCodeError for std::io::Error {
    fn to_exit_code(&self) -> ExitCode {
        use std::io::ErrorKind;
        match self.kind() {
            ErrorKind::AddrNotAvailable => ExitCode::NoHost,
            ErrorKind::AddrInUse
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::NotConnected => ExitCode::Unavailable,
            ErrorKind::InvalidData => ExitCode::DataErr,
            ErrorKind::TimedOut => ExitCode::TempFail,
            _ => ExitCode::IoError,
        }
    }
}

impl<T: ExitCodeError + 'static> From<T> for Box<dyn ExitCodeError> {
    fn from(x: T) -> Self {
        Box::new(x)
    }
}

/// Represents an error containing an explicit exit code associated with some error
#[derive(Debug, Display, Error)]
#[display(fmt = "{}", error)]
pub struct WrappedExitCodeError {
    #[error(ignore)]
    exit_code: ExitCode,

    error: Box<dyn Error + Send + Sync>,
}

impl WrappedExitCodeError {
    pub fn new(
        exit_code: impl Into<ExitCode>,
        error: impl Into<Box<dyn Error + Send + Sync>>,
    ) -> Self {
        Self {
            error: error.into(),
            exit_code: exit_code.into(),
        }
    }

    pub fn exit_code(&self) -> ExitCode {
        self.exit_code
    }
}

impl ExitCodeError for WrappedExitCodeError {
    fn to_exit_code(&self) -> ExitCode {
        self.exit_code
    }
}
