use derive_more::Display;
use notify::ErrorKind as NotifyErrorKind;
use serde::{Deserialize, Serialize};
use std::io;

/// General purpose error type that can be sent across the wire
#[derive(Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[display(fmt = "{}: {}", kind, description)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Error {
    /// Label describing the kind of error
    pub kind: ErrorKind,

    /// Description of the error itself
    pub description: String,
}

impl std::error::Error for Error {}

#[cfg(feature = "schemars")]
impl Error {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Error)
    }
}

impl<'a> From<&'a str> for Error {
    fn from(x: &'a str) -> Self {
        Self::from(x.to_string())
    }
}

impl From<String> for Error {
    fn from(x: String) -> Self {
        Self {
            kind: ErrorKind::Other,
            description: x,
        }
    }
}

impl From<io::Error> for Error {
    fn from(x: io::Error) -> Self {
        Self {
            kind: ErrorKind::from(x.kind()),
            description: x.to_string(),
        }
    }
}

impl From<Error> for io::Error {
    fn from(x: Error) -> Self {
        Self::new(x.kind.into(), x.description)
    }
}

impl From<notify::Error> for Error {
    fn from(x: notify::Error) -> Self {
        let err = match x.kind {
            NotifyErrorKind::Generic(x) => Self {
                kind: ErrorKind::Other,
                description: x,
            },
            NotifyErrorKind::Io(x) => Self::from(x),
            NotifyErrorKind::PathNotFound => Self {
                kind: ErrorKind::Other,
                description: String::from("Path not found"),
            },
            NotifyErrorKind::WatchNotFound => Self {
                kind: ErrorKind::Other,
                description: String::from("Watch not found"),
            },
            NotifyErrorKind::InvalidConfig(_) => Self {
                kind: ErrorKind::Other,
                description: String::from("Invalid config"),
            },
            NotifyErrorKind::MaxFilesWatch => Self {
                kind: ErrorKind::Other,
                description: String::from("Max files watched"),
            },
        };

        Self {
            kind: err.kind,
            description: format!(
                "{}\n\nPaths: {}",
                err.description,
                x.paths
                    .into_iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
        }
    }
}

impl From<walkdir::Error> for Error {
    fn from(x: walkdir::Error) -> Self {
        if x.io_error().is_some() {
            x.into_io_error().map(Self::from).unwrap()
        } else {
            Self {
                kind: ErrorKind::Loop,
                description: format!("{}", x),
            }
        }
    }
}

impl From<tokio::task::JoinError> for Error {
    fn from(x: tokio::task::JoinError) -> Self {
        Self {
            kind: if x.is_cancelled() {
                ErrorKind::TaskCancelled
            } else {
                ErrorKind::TaskPanicked
            },
            description: format!("{}", x),
        }
    }
}

/// All possible kinds of errors that can be returned
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ErrorKind {
    /// An entity was not found, often a file
    NotFound,

    /// The operation lacked the necessary privileges to complete
    PermissionDenied,

    /// The connection was refused by the remote server
    ConnectionRefused,

    /// The connection was reset by the remote server
    ConnectionReset,

    /// The connection was aborted (terminated) by the remote server
    ConnectionAborted,

    /// The network operation failed because it was not connected yet
    NotConnected,

    /// A socket address could not be bound because the address is already in use elsewhere
    AddrInUse,

    /// A nonexistent interface was requested or the requested address was not local
    AddrNotAvailable,

    /// The operation failed because a pipe was closed
    BrokenPipe,

    /// An entity already exists, often a file
    AlreadyExists,

    /// The operation needs to block to complete, but the blocking operation was requested to not
    /// occur
    WouldBlock,

    /// A parameter was incorrect
    InvalidInput,

    /// Data not valid for the operation were encountered
    InvalidData,

    /// The I/O operation's timeout expired, causing it to be cancelled
    TimedOut,

    /// An error returned when an operation could not be completed because a
    /// call to `write` returned `Ok(0)`
    WriteZero,

    /// This operation was interrupted
    Interrupted,

    /// Any I/O error not part of this list
    Other,

    /// An error returned when an operation could not be completed because an "end of file" was
    /// reached prematurely
    UnexpectedEof,

    /// This operation is unsupported on this platform
    Unsupported,

    /// An operation could not be completed, because it failed to allocate enough memory
    OutOfMemory,

    /// When a loop is encountered when walking a directory
    Loop,

    /// When a task is cancelled
    TaskCancelled,

    /// When a task panics
    TaskPanicked,

    /// Catchall for an error that has no specific type
    Unknown,
}

#[cfg(feature = "schemars")]
impl ErrorKind {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(ErrorKind)
    }
}

impl From<io::ErrorKind> for ErrorKind {
    fn from(kind: io::ErrorKind) -> Self {
        match kind {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::ConnectionRefused => Self::ConnectionRefused,
            io::ErrorKind::ConnectionReset => Self::ConnectionReset,
            io::ErrorKind::ConnectionAborted => Self::ConnectionAborted,
            io::ErrorKind::NotConnected => Self::NotConnected,
            io::ErrorKind::AddrInUse => Self::AddrInUse,
            io::ErrorKind::AddrNotAvailable => Self::AddrNotAvailable,
            io::ErrorKind::BrokenPipe => Self::BrokenPipe,
            io::ErrorKind::AlreadyExists => Self::AlreadyExists,
            io::ErrorKind::WouldBlock => Self::WouldBlock,
            io::ErrorKind::InvalidInput => Self::InvalidInput,
            io::ErrorKind::InvalidData => Self::InvalidData,
            io::ErrorKind::TimedOut => Self::TimedOut,
            io::ErrorKind::WriteZero => Self::WriteZero,
            io::ErrorKind::Interrupted => Self::Interrupted,
            io::ErrorKind::Other => Self::Other,
            io::ErrorKind::OutOfMemory => Self::OutOfMemory,
            io::ErrorKind::UnexpectedEof => Self::UnexpectedEof,
            io::ErrorKind::Unsupported => Self::Unsupported,

            // This exists because io::ErrorKind is non_exhaustive
            _ => Self::Unknown,
        }
    }
}

impl From<ErrorKind> for io::ErrorKind {
    fn from(kind: ErrorKind) -> Self {
        match kind {
            ErrorKind::NotFound => Self::NotFound,
            ErrorKind::PermissionDenied => Self::PermissionDenied,
            ErrorKind::ConnectionRefused => Self::ConnectionRefused,
            ErrorKind::ConnectionReset => Self::ConnectionReset,
            ErrorKind::ConnectionAborted => Self::ConnectionAborted,
            ErrorKind::NotConnected => Self::NotConnected,
            ErrorKind::AddrInUse => Self::AddrInUse,
            ErrorKind::AddrNotAvailable => Self::AddrNotAvailable,
            ErrorKind::BrokenPipe => Self::BrokenPipe,
            ErrorKind::AlreadyExists => Self::AlreadyExists,
            ErrorKind::WouldBlock => Self::WouldBlock,
            ErrorKind::InvalidInput => Self::InvalidInput,
            ErrorKind::InvalidData => Self::InvalidData,
            ErrorKind::TimedOut => Self::TimedOut,
            ErrorKind::WriteZero => Self::WriteZero,
            ErrorKind::Interrupted => Self::Interrupted,
            ErrorKind::Other => Self::Other,
            ErrorKind::OutOfMemory => Self::OutOfMemory,
            ErrorKind::UnexpectedEof => Self::UnexpectedEof,
            ErrorKind::Unsupported => Self::Unsupported,
            _ => Self::Other,
        }
    }
}
