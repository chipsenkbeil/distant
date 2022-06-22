use derive_more::Display;
use serde::{Deserialize, Serialize};
use std::io;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Error {
    pub kind: ErrorKind,
    pub description: String,
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
        io::Error::new(x.kind.into(), x.description)
    }
}

#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Catchall for an error that has no specific type
    Unknown,
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
