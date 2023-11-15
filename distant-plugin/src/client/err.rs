use std::error;
use std::fmt;
use std::io;

use crate::common::Id;
use crate::protocol;

pub type ClientResult<T> = Result<T, ClientError>;

/// Errors that can occur from sending data using a client.
#[derive(Debug)]
pub enum ClientError {
    /// A networking error occurred when trying to submit the request.
    Io(io::Error),

    /// An error occurred server-side.
    Server(protocol::Error),

    /// A response was received, but its origin did not match the request.
    WrongOrigin { expected: Id, actual: Id },

    /// A response was received, but the payload was single when expected batch or vice versa.
    WrongPayloadFormat,

    /// A response was received, but its payload type did not match any expected response type.
    WrongPayloadType {
        expected: &'static [&'static str],
        actual: &'static str,
    },
}

impl error::Error for ClientError {}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(x) => fmt::Display::fmt(x, f),
            Self::Server(x) => fmt::Display::fmt(x, f),
            Self::WrongOrigin { expected, actual } => {
                write!(
                    f,
                    "Wrong response origin! Expected {expected}, got {actual}."
                )
            }
            Self::WrongPayloadFormat => write!(f, "Wrong response payload format!"),
            Self::WrongPayloadType { expected, actual } => {
                if expected.len() == 1 {
                    let expected = expected[0];
                    write!(
                        f,
                        "Wrong response payload type! Wanted {expected}, but got {actual}."
                    )
                } else {
                    let expected = expected.join(",");
                    write!(
                        f,
                        "Wrong response type! Wanted one of {expected}, but got {actual}."
                    )
                }
            }
        }
    }
}

impl From<io::Error> for ClientError {
    fn from(x: io::Error) -> Self {
        Self::Io(x)
    }
}

impl From<protocol::Error> for ClientError {
    fn from(x: protocol::Error) -> Self {
        Self::Server(x)
    }
}
