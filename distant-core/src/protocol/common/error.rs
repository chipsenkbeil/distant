use std::io;

use derive_more::Display;
use serde::{Deserialize, Serialize};

/// General purpose error type that can be sent across the wire
#[derive(Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[display(fmt = "{kind}: {description}")]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Error {
    /// Label describing the kind of error
    pub kind: ErrorKind,

    /// Description of the error itself
    pub description: String,
}

impl std::error::Error for Error {}

impl Error {
    /// Produces an [`io::Error`] from this error.
    pub fn to_io_error(&self) -> io::Error {
        io::Error::new(self.kind.into(), self.description.to_string())
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

/// All possible kinds of errors that can be returned
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

    /// When a loop is encountered when walking a directory
    Loop,

    /// When a task is cancelled
    TaskCancelled,

    /// When a task panics
    TaskPanicked,

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

#[cfg(test)]
mod tests {
    use super::*;

    mod error {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let error = Error {
                kind: ErrorKind::AddrInUse,
                description: "some description".to_string(),
            };

            let value = serde_json::to_value(error).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "kind": "addr_in_use",
                    "description": "some description",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "kind": "addr_in_use",
                "description": "some description",
            });

            let error: Error = serde_json::from_value(value).unwrap();
            assert_eq!(
                error,
                Error {
                    kind: ErrorKind::AddrInUse,
                    description: "some description".to_string(),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let error = Error {
                kind: ErrorKind::AddrInUse,
                description: "some description".to_string(),
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&error).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or preventing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Error {
                kind: ErrorKind::AddrInUse,
                description: "some description".to_string(),
            })
            .unwrap();

            let error: Error = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                error,
                Error {
                    kind: ErrorKind::AddrInUse,
                    description: "some description".to_string(),
                }
            );
        }
    }

    mod error_kind {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let kind = ErrorKind::AddrInUse;

            let value = serde_json::to_value(kind).unwrap();
            assert_eq!(value, serde_json::json!("addr_in_use"));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!("addr_in_use");

            let kind: ErrorKind = serde_json::from_value(value).unwrap();
            assert_eq!(kind, ErrorKind::AddrInUse);
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let kind = ErrorKind::AddrInUse;

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&kind).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&ErrorKind::AddrInUse).unwrap();

            let kind: ErrorKind = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(kind, ErrorKind::AddrInUse);
        }

        #[test]
        fn roundtrip_through_io_error_kind_for_all_mapped_variants() {
            let pairs: &[(ErrorKind, io::ErrorKind)] = &[
                (ErrorKind::NotFound, io::ErrorKind::NotFound),
                (ErrorKind::PermissionDenied, io::ErrorKind::PermissionDenied),
                (
                    ErrorKind::ConnectionRefused,
                    io::ErrorKind::ConnectionRefused,
                ),
                (ErrorKind::ConnectionReset, io::ErrorKind::ConnectionReset),
                (
                    ErrorKind::ConnectionAborted,
                    io::ErrorKind::ConnectionAborted,
                ),
                (ErrorKind::NotConnected, io::ErrorKind::NotConnected),
                (ErrorKind::AddrInUse, io::ErrorKind::AddrInUse),
                (ErrorKind::AddrNotAvailable, io::ErrorKind::AddrNotAvailable),
                (ErrorKind::BrokenPipe, io::ErrorKind::BrokenPipe),
                (ErrorKind::AlreadyExists, io::ErrorKind::AlreadyExists),
                (ErrorKind::WouldBlock, io::ErrorKind::WouldBlock),
                (ErrorKind::InvalidInput, io::ErrorKind::InvalidInput),
                (ErrorKind::InvalidData, io::ErrorKind::InvalidData),
                (ErrorKind::TimedOut, io::ErrorKind::TimedOut),
                (ErrorKind::WriteZero, io::ErrorKind::WriteZero),
                (ErrorKind::Interrupted, io::ErrorKind::Interrupted),
                (ErrorKind::Other, io::ErrorKind::Other),
                (ErrorKind::UnexpectedEof, io::ErrorKind::UnexpectedEof),
                (ErrorKind::Unsupported, io::ErrorKind::Unsupported),
                (ErrorKind::OutOfMemory, io::ErrorKind::OutOfMemory),
            ];

            for &(error_kind, io_kind) in pairs {
                // ErrorKind -> io::ErrorKind
                let converted_io: io::ErrorKind = error_kind.into();
                assert_eq!(
                    converted_io, io_kind,
                    "ErrorKind::{error_kind} -> io::ErrorKind failed"
                );

                // io::ErrorKind -> ErrorKind
                let converted_back: ErrorKind = io_kind.into();
                assert_eq!(
                    converted_back, error_kind,
                    "io::ErrorKind -> ErrorKind::{error_kind} failed"
                );
            }
        }

        #[test]
        fn variants_without_io_mapping_should_convert_to_io_other() {
            let unmapped = [
                ErrorKind::Loop,
                ErrorKind::TaskCancelled,
                ErrorKind::TaskPanicked,
                ErrorKind::Unknown,
            ];
            for kind in unmapped {
                let io_kind: io::ErrorKind = kind.into();
                assert_eq!(
                    io_kind,
                    io::ErrorKind::Other,
                    "ErrorKind::{kind} should map to io::ErrorKind::Other"
                );
            }
        }
    }

    mod error_construction {
        use super::*;

        #[test]
        fn from_str_should_create_error_with_other_kind() {
            let error = Error::from("something failed");
            assert_eq!(error.kind, ErrorKind::Other);
            assert_eq!(error.description, "something failed");
        }

        #[test]
        fn from_string_should_create_error_with_other_kind() {
            let error = Error::from(String::from("something failed"));
            assert_eq!(error.kind, ErrorKind::Other);
            assert_eq!(error.description, "something failed");
        }

        #[test]
        fn from_io_error_should_preserve_kind() {
            let io_err = io::Error::new(io::ErrorKind::NotFound, "file missing");
            let error = Error::from(io_err);
            assert_eq!(error.kind, ErrorKind::NotFound);
            assert!(error.description.contains("file missing"));
        }

        #[test]
        fn into_io_error_should_preserve_kind_and_description() {
            let error = Error {
                kind: ErrorKind::PermissionDenied,
                description: "access denied".to_string(),
            };
            let io_err: io::Error = error.into();
            assert_eq!(io_err.kind(), io::ErrorKind::PermissionDenied);
            assert!(io_err.to_string().contains("access denied"));
        }

        #[test]
        fn to_io_error_should_create_io_error_with_matching_kind() {
            let error = Error {
                kind: ErrorKind::TimedOut,
                description: "deadline exceeded".to_string(),
            };
            let io_err = error.to_io_error();
            assert_eq!(io_err.kind(), io::ErrorKind::TimedOut);
            assert!(io_err.to_string().contains("deadline exceeded"));
        }

        #[test]
        fn display_should_show_kind_and_description() {
            let error = Error {
                kind: ErrorKind::NotFound,
                description: "file not found".to_string(),
            };
            let displayed = error.to_string();
            assert!(displayed.contains("NotFound"));
            assert!(displayed.contains("file not found"));
        }

        #[test]
        fn json_roundtrip_for_all_error_kinds() {
            let all_kinds = [
                ErrorKind::NotFound,
                ErrorKind::PermissionDenied,
                ErrorKind::ConnectionRefused,
                ErrorKind::ConnectionReset,
                ErrorKind::ConnectionAborted,
                ErrorKind::NotConnected,
                ErrorKind::AddrInUse,
                ErrorKind::AddrNotAvailable,
                ErrorKind::BrokenPipe,
                ErrorKind::AlreadyExists,
                ErrorKind::WouldBlock,
                ErrorKind::InvalidInput,
                ErrorKind::InvalidData,
                ErrorKind::TimedOut,
                ErrorKind::WriteZero,
                ErrorKind::Interrupted,
                ErrorKind::Other,
                ErrorKind::UnexpectedEof,
                ErrorKind::Unsupported,
                ErrorKind::OutOfMemory,
                ErrorKind::Loop,
                ErrorKind::TaskCancelled,
                ErrorKind::TaskPanicked,
                ErrorKind::Unknown,
            ];

            for kind in all_kinds {
                let error = Error {
                    kind,
                    description: format!("test {kind}"),
                };
                let json = serde_json::to_value(&error).unwrap();
                let restored: Error = serde_json::from_value(json).unwrap();
                assert_eq!(restored, error);
            }
        }
    }
}
