use distant_core::{RemoteProcessError, TransportError};

/// Exit codes following https://www.freebsd.org/cgi/man.cgi?query=sysexits&sektion=3
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub enum ExitCode {
    /// EX_USAGE (64) - being used when arguments missing or bad arguments provided to CLI
    Usage,

    /// EX_DATAERR (65) - being used when bad data received not in UTF-8 format or transport data
    /// is bad
    DataErr,

    /// EX_NOINPUT (66) - being used when not getting expected data from launch
    NoInput,

    /// EX_NOHOST (68) - being used when failed to resolve a host
    NoHost,

    /// EX_UNAVAILABLE (69) - being used when IO error encountered where connection is problem
    Unavailable,

    /// EX_SOFTWARE (70) - being used for internal errors that can occur like joining a task
    Software,

    /// EX_OSERR (71) - being used when fork failed
    OsErr,

    /// EX_IOERR (74) - being used as catchall for IO errors
    IoError,

    /// EX_TEMPFAIL (75) - being used when we get a timeout
    TempFail,

    /// EX_PROTOCOL (76) - being used as catchall for transport errors
    Protocol,

    /// Custom exit code to pass back verbatim
    Custom(i32),
}

/// Represents an error that can be converted into an exit code
pub trait ExitCodeError: std::error::Error {
    fn to_exit_code(&self) -> ExitCode;

    fn to_i32(&self) -> i32 {
        match self.to_exit_code() {
            ExitCode::Usage => 64,
            ExitCode::DataErr => 65,
            ExitCode::NoInput => 66,
            ExitCode::NoHost => 68,
            ExitCode::Unavailable => 69,
            ExitCode::Software => 70,
            ExitCode::OsErr => 71,
            ExitCode::IoError => 74,
            ExitCode::TempFail => 75,
            ExitCode::Protocol => 76,
            ExitCode::Custom(x) => x,
        }
    }
}

impl ExitCodeError for std::io::Error {
    fn to_exit_code(&self) -> ExitCode {
        use std::io::ErrorKind;
        match self.kind() {
            ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::NotConnected => ExitCode::Unavailable,
            ErrorKind::InvalidData => ExitCode::DataErr,
            ErrorKind::TimedOut => ExitCode::TempFail,
            _ => ExitCode::IoError,
        }
    }
}

impl ExitCodeError for TransportError {
    fn to_exit_code(&self) -> ExitCode {
        match self {
            TransportError::IoError(x) => x.to_exit_code(),
            _ => ExitCode::Protocol,
        }
    }
}

impl ExitCodeError for RemoteProcessError {
    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::BadResponse => ExitCode::DataErr,
            Self::ChannelDead => ExitCode::Unavailable,
            Self::TransportError(x) => x.to_exit_code(),
            Self::UnexpectedEof => ExitCode::IoError,
            Self::WaitFailed(_) => ExitCode::Software,
        }
    }
}

impl<T: ExitCodeError + 'static> From<T> for Box<dyn ExitCodeError> {
    fn from(x: T) -> Self {
        Box::new(x)
    }
}