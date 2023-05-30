use std::io;

use derive_more::IsVariant;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumDiscriminants, EnumIter, EnumMessage, EnumString};

use crate::common::{
    Capabilities, Change, DirEntry, Error, Metadata, ProcessId, SearchId, SearchQueryMatch,
    SystemInfo,
};

/// Represents the payload of a successful response
#[derive(
    Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, EnumDiscriminants, Serialize, Deserialize,
)]
#[strum_discriminants(derive(
    AsRefStr,
    strum::Display,
    EnumIter,
    EnumMessage,
    EnumString,
    Hash,
    PartialOrd,
    Ord,
    IsVariant,
    Serialize,
    Deserialize
))]
#[strum_discriminants(name(ResponseKind))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
#[strum(serialize_all = "snake_case")]
pub enum Response {
    /// General okay with no extra data, returned in cases like
    /// creating or removing a directory, copying a file, or renaming
    /// a file
    Ok,

    /// General-purpose failure that occurred from some request
    Error(Error),

    /// Response containing some arbitrary, binary data
    Blob {
        /// Binary data associated with the response
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },

    /// Response containing some arbitrary, text data
    Text {
        /// Text data associated with the response
        data: String,
    },

    /// Response to reading a directory
    DirEntries {
        /// Entries contained within the requested directory
        entries: Vec<DirEntry>,

        /// Errors encountered while scanning for entries
        errors: Vec<Error>,
    },

    /// Response to a filesystem change for some watched file, directory, or symlink
    Changed(Change),

    /// Response to checking if a path exists
    Exists { value: bool },

    /// Represents metadata about some filesystem object (file, directory, symlink) on remote machine
    Metadata(Metadata),

    /// Represents a search being started
    SearchStarted {
        /// Arbitrary id associated with search
        id: SearchId,
    },

    /// Represents some subset of results for a search query (may not be all of them)
    SearchResults {
        /// Arbitrary id associated with search
        id: SearchId,

        /// Collection of matches from performing a query
        matches: Vec<SearchQueryMatch>,
    },

    /// Represents a search being completed
    SearchDone {
        /// Arbitrary id associated with search
        id: SearchId,
    },

    /// Response to starting a new process
    ProcSpawned {
        /// Arbitrary id associated with running process
        id: ProcessId,
    },

    /// Actively-transmitted stdout as part of running process
    ProcStdout {
        /// Arbitrary id associated with running process
        id: ProcessId,

        /// Data read from a process' stdout pipe
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },

    /// Actively-transmitted stderr as part of running process
    ProcStderr {
        /// Arbitrary id associated with running process
        id: ProcessId,

        /// Data read from a process' stderr pipe
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },

    /// Response to a process finishing
    ProcDone {
        /// Arbitrary id associated with running process
        id: ProcessId,

        /// Whether or not termination was successful
        success: bool,

        /// Exit code associated with termination, will be missing if terminated by signal
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<i32>,
    },

    /// Response to retrieving information about the server and the system it is on
    SystemInfo(SystemInfo),

    /// Response to retrieving information about the server's capabilities
    Capabilities { supported: Capabilities },
}

impl From<io::Error> for Response {
    fn from(x: io::Error) -> Self {
        Self::Error(Error::from(x))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod ok {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Response::Ok;

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "ok",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "ok",
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(payload, Response::Ok);
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Response::Ok;

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::Ok).unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(payload, Response::Ok);
        }
    }

    mod error {
        use super::*;
        use crate::common::ErrorKind;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Response::Error(Error {
                kind: ErrorKind::AddrInUse,
                description: String::from("some description"),
            });

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "error",
                    "kind": "addr_in_use",
                    "description": "some description",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "error",
                "kind": "addr_in_use",
                "description": "some description",
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Response::Error(Error {
                    kind: ErrorKind::AddrInUse,
                    description: String::from("some description"),
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Response::Error(Error {
                kind: ErrorKind::AddrInUse,
                description: String::from("some description"),
            });

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::Error(Error {
                kind: ErrorKind::AddrInUse,
                description: String::from("some description"),
            }))
            .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Response::Error(Error {
                    kind: ErrorKind::AddrInUse,
                    description: String::from("some description"),
                })
            );
        }
    }

    mod blob {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Response::Blob {
                data: vec![0, 1, 2, u8::MAX],
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "blob",
                    "data": [0, 1, 2, u8::MAX],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "blob",
                "data": [0, 1, 2, u8::MAX],
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Response::Blob {
                    data: vec![0, 1, 2, u8::MAX],
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Response::Blob {
                data: vec![0, 1, 2, u8::MAX],
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::Blob {
                data: vec![0, 1, 2, u8::MAX],
            })
            .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Response::Blob {
                    data: vec![0, 1, 2, u8::MAX],
                }
            );
        }
    }

    mod text {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Response::Text {
                data: String::from("some text"),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "text",
                    "data": "some text",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "text",
                "data": "some text",
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Response::Text {
                    data: String::from("some text"),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Response::Text {
                data: String::from("some text"),
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::Text {
                data: String::from("some text"),
            })
            .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Response::Text {
                    data: String::from("some text"),
                }
            );
        }
    }

    mod dir_entries {
        use std::path::PathBuf;

        use super::*;
        use crate::common::{ErrorKind, FileType};

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Response::DirEntries {
                entries: Vec::new(),
                errors: Vec::new(),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "dir_entries",
                    "entries": [],
                    "errors": [],
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Response::DirEntries {
                entries: vec![DirEntry {
                    path: PathBuf::from("path"),
                    file_type: FileType::File,
                    depth: usize::MAX,
                }],
                errors: vec![Error {
                    kind: ErrorKind::AddrInUse,
                    description: String::from("some description"),
                }],
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "dir_entries",
                    "entries": [{
                        "path": "path",
                        "file_type": "file",
                        "depth": usize::MAX,
                    }],
                    "errors": [{
                        "kind": "addr_in_use",
                        "description": "some description",
                    }],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "dir_entries",
                "entries": [],
                "errors": [],
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Response::DirEntries {
                    entries: Vec::new(),
                    errors: Vec::new(),
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let value = serde_json::json!({
                "type": "dir_entries",
                "entries": [{
                    "path": "path",
                    "file_type": "file",
                    "depth": usize::MAX,
                }],
                "errors": [{
                    "kind": "addr_in_use",
                    "description": "some description",
                }],
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Response::DirEntries {
                    entries: vec![DirEntry {
                        path: PathBuf::from("path"),
                        file_type: FileType::File,
                        depth: usize::MAX,
                    }],
                    errors: vec![Error {
                        kind: ErrorKind::AddrInUse,
                        description: String::from("some description"),
                    }],
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Response::DirEntries {
                entries: Vec::new(),
                errors: Vec::new(),
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Response::DirEntries {
                entries: vec![DirEntry {
                    path: PathBuf::from("path"),
                    file_type: FileType::File,
                    depth: usize::MAX,
                }],
                errors: vec![Error {
                    kind: ErrorKind::AddrInUse,
                    description: String::from("some description"),
                }],
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::DirEntries {
                entries: Vec::new(),
                errors: Vec::new(),
            })
            .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Response::DirEntries {
                    entries: Vec::new(),
                    errors: Vec::new(),
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::DirEntries {
                entries: vec![DirEntry {
                    path: PathBuf::from("path"),
                    file_type: FileType::File,
                    depth: usize::MAX,
                }],
                errors: vec![Error {
                    kind: ErrorKind::AddrInUse,
                    description: String::from("some description"),
                }],
            })
            .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Response::DirEntries {
                    entries: vec![DirEntry {
                        path: PathBuf::from("path"),
                        file_type: FileType::File,
                        depth: usize::MAX,
                    }],
                    errors: vec![Error {
                        kind: ErrorKind::AddrInUse,
                        description: String::from("some description"),
                    }],
                }
            );
        }
    }

    mod changed {
        use super::*;
        use crate::common::ChangeKind;
        use std::path::PathBuf;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Response::Changed(Change {
                kind: ChangeKind::Access,
                paths: vec![PathBuf::from("path")],
            });

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "changed",
                    "kind": "access",
                    "paths": ["path"],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "changed",
                "kind": "access",
                "paths": ["path"],
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Response::Changed(Change {
                    kind: ChangeKind::Access,
                    paths: vec![PathBuf::from("path")],
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Response::Changed(Change {
                kind: ChangeKind::Access,
                paths: vec![PathBuf::from("path")],
            });

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::Changed(Change {
                kind: ChangeKind::Access,
                paths: vec![PathBuf::from("path")],
            }))
            .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Response::Changed(Change {
                    kind: ChangeKind::Access,
                    paths: vec![PathBuf::from("path")],
                })
            );
        }
    }

    mod exists {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Response::Exists { value: true };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "exists",
                    "value": true,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "exists",
                "value": true,
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(payload, Response::Exists { value: true });
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Response::Exists { value: true };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::Exists { value: true }).unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(payload, Response::Exists { value: true });
        }
    }

    mod metadata {
        use super::*;
        use crate::common::{FileType, UnixMetadata, WindowsMetadata};
        use std::path::PathBuf;

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Response::Metadata(Metadata {
                canonicalized_path: None,
                file_type: FileType::File,
                len: 0,
                readonly: false,
                accessed: None,
                created: None,
                modified: None,
                unix: None,
                windows: None,
            });

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "metadata",
                    "file_type": "file",
                    "len": 0,
                    "readonly": false,
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Response::Metadata(Metadata {
                canonicalized_path: Some(PathBuf::from("path")),
                file_type: FileType::File,
                len: u64::MAX,
                readonly: true,
                accessed: Some(u128::MAX),
                created: Some(u128::MAX),
                modified: Some(u128::MAX),
                unix: Some(UnixMetadata {
                    owner_read: true,
                    owner_write: false,
                    owner_exec: false,
                    group_read: true,
                    group_write: false,
                    group_exec: false,
                    other_read: true,
                    other_write: false,
                    other_exec: false,
                }),
                windows: Some(WindowsMetadata {
                    archive: true,
                    compressed: false,
                    encrypted: true,
                    hidden: false,
                    integrity_stream: true,
                    normal: false,
                    not_content_indexed: true,
                    no_scrub_data: false,
                    offline: true,
                    recall_on_data_access: false,
                    recall_on_open: true,
                    reparse_point: false,
                    sparse_file: true,
                    system: false,
                    temporary: true,
                }),
            });

            // NOTE: These values are too big to normally serialize, so we have to convert them to
            // a string type, which is why the value here also needs to be a string.
            let u128_max_str = u128::MAX.to_string();

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "metadata",
                    "canonicalized_path": "path",
                    "file_type": "file",
                    "len": u64::MAX,
                    "readonly": true,
                    "accessed": u128_max_str,
                    "created": u128_max_str,
                    "modified": u128_max_str,
                    "unix": {
                        "owner_read": true,
                        "owner_write": false,
                        "owner_exec": false,
                        "group_read": true,
                        "group_write": false,
                        "group_exec": false,
                        "other_read": true,
                        "other_write": false,
                        "other_exec": false,
                    },
                    "windows": {
                        "archive": true,
                        "compressed": false,
                        "encrypted": true,
                        "hidden": false,
                        "integrity_stream": true,
                        "normal": false,
                        "not_content_indexed": true,
                        "no_scrub_data": false,
                        "offline": true,
                        "recall_on_data_access": false,
                        "recall_on_open": true,
                        "reparse_point": false,
                        "sparse_file": true,
                        "system": false,
                        "temporary": true,
                    }
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "metadata",
                "file_type": "file",
                "len": 0,
                "readonly": false,
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Response::Metadata(Metadata {
                    canonicalized_path: None,
                    file_type: FileType::File,
                    len: 0,
                    readonly: false,
                    accessed: None,
                    created: None,
                    modified: None,
                    unix: None,
                    windows: None,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let u128_max_str = u128::MAX.to_string();
            let value = serde_json::json!({
                "type": "metadata",
                "canonicalized_path": "path",
                "file_type": "file",
                "len": u64::MAX,
                "readonly": true,
                "accessed": u128_max_str,
                "created": u128_max_str,
                "modified": u128_max_str,
                "unix": {
                    "owner_read": true,
                    "owner_write": false,
                    "owner_exec": false,
                    "group_read": true,
                    "group_write": false,
                    "group_exec": false,
                    "other_read": true,
                    "other_write": false,
                    "other_exec": false,
                },
                "windows": {
                    "archive": true,
                    "compressed": false,
                    "encrypted": true,
                    "hidden": false,
                    "integrity_stream": true,
                    "normal": false,
                    "not_content_indexed": true,
                    "no_scrub_data": false,
                    "offline": true,
                    "recall_on_data_access": false,
                    "recall_on_open": true,
                    "reparse_point": false,
                    "sparse_file": true,
                    "system": false,
                    "temporary": true,
                }
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Response::Metadata(Metadata {
                    canonicalized_path: Some(PathBuf::from("path")),
                    file_type: FileType::File,
                    len: u64::MAX,
                    readonly: true,
                    accessed: Some(u128::MAX),
                    created: Some(u128::MAX),
                    modified: Some(u128::MAX),
                    unix: Some(UnixMetadata {
                        owner_read: true,
                        owner_write: false,
                        owner_exec: false,
                        group_read: true,
                        group_write: false,
                        group_exec: false,
                        other_read: true,
                        other_write: false,
                        other_exec: false,
                    }),
                    windows: Some(WindowsMetadata {
                        archive: true,
                        compressed: false,
                        encrypted: true,
                        hidden: false,
                        integrity_stream: true,
                        normal: false,
                        not_content_indexed: true,
                        no_scrub_data: false,
                        offline: true,
                        recall_on_data_access: false,
                        recall_on_open: true,
                        reparse_point: false,
                        sparse_file: true,
                        system: false,
                        temporary: true,
                    }),
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Response::Metadata(Metadata {
                canonicalized_path: None,
                file_type: FileType::File,
                len: 0,
                readonly: false,
                accessed: None,
                created: None,
                modified: None,
                unix: None,
                windows: None,
            });

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Response::Metadata(Metadata {
                canonicalized_path: Some(PathBuf::from("path")),
                file_type: FileType::File,
                len: u64::MAX,
                readonly: true,
                accessed: Some(u128::MAX),
                created: Some(u128::MAX),
                modified: Some(u128::MAX),
                unix: Some(UnixMetadata {
                    owner_read: true,
                    owner_write: false,
                    owner_exec: false,
                    group_read: true,
                    group_write: false,
                    group_exec: false,
                    other_read: true,
                    other_write: false,
                    other_exec: false,
                }),
                windows: Some(WindowsMetadata {
                    archive: true,
                    compressed: false,
                    encrypted: true,
                    hidden: false,
                    integrity_stream: true,
                    normal: false,
                    not_content_indexed: true,
                    no_scrub_data: false,
                    offline: true,
                    recall_on_data_access: false,
                    recall_on_open: true,
                    reparse_point: false,
                    sparse_file: true,
                    system: false,
                    temporary: true,
                }),
            });

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::Metadata(Metadata {
                canonicalized_path: None,
                file_type: FileType::File,
                len: 0,
                readonly: false,
                accessed: None,
                created: None,
                modified: None,
                unix: None,
                windows: None,
            }))
            .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Response::Metadata(Metadata {
                    canonicalized_path: None,
                    file_type: FileType::File,
                    len: 0,
                    readonly: false,
                    accessed: None,
                    created: None,
                    modified: None,
                    unix: None,
                    windows: None,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::Metadata(Metadata {
                canonicalized_path: Some(PathBuf::from("path")),
                file_type: FileType::File,
                len: u64::MAX,
                readonly: true,
                accessed: Some(u128::MAX),
                created: Some(u128::MAX),
                modified: Some(u128::MAX),
                unix: Some(UnixMetadata {
                    owner_read: true,
                    owner_write: false,
                    owner_exec: false,
                    group_read: true,
                    group_write: false,
                    group_exec: false,
                    other_read: true,
                    other_write: false,
                    other_exec: false,
                }),
                windows: Some(WindowsMetadata {
                    archive: true,
                    compressed: false,
                    encrypted: true,
                    hidden: false,
                    integrity_stream: true,
                    normal: false,
                    not_content_indexed: true,
                    no_scrub_data: false,
                    offline: true,
                    recall_on_data_access: false,
                    recall_on_open: true,
                    reparse_point: false,
                    sparse_file: true,
                    system: false,
                    temporary: true,
                }),
            }))
            .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Response::Metadata(Metadata {
                    canonicalized_path: Some(PathBuf::from("path")),
                    file_type: FileType::File,
                    len: u64::MAX,
                    readonly: true,
                    accessed: Some(u128::MAX),
                    created: Some(u128::MAX),
                    modified: Some(u128::MAX),
                    unix: Some(UnixMetadata {
                        owner_read: true,
                        owner_write: false,
                        owner_exec: false,
                        group_read: true,
                        group_write: false,
                        group_exec: false,
                        other_read: true,
                        other_write: false,
                        other_exec: false,
                    }),
                    windows: Some(WindowsMetadata {
                        archive: true,
                        compressed: false,
                        encrypted: true,
                        hidden: false,
                        integrity_stream: true,
                        normal: false,
                        not_content_indexed: true,
                        no_scrub_data: false,
                        offline: true,
                        recall_on_data_access: false,
                        recall_on_open: true,
                        reparse_point: false,
                        sparse_file: true,
                        system: false,
                        temporary: true,
                    }),
                })
            );
        }
    }

    mod search_started {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Response::SearchStarted { id: SearchId::MAX };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "search_started",
                    "id": SearchId::MAX,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "search_started",
                "id": SearchId::MAX,
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(payload, Response::SearchStarted { id: SearchId::MAX });
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Response::SearchStarted { id: SearchId::MAX };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf =
                rmp_serde::encode::to_vec_named(&Response::SearchStarted { id: SearchId::MAX })
                    .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(payload, Response::SearchStarted { id: SearchId::MAX });
        }
    }

    mod search_results {
        use super::*;
        use crate::common::{SearchQueryContentsMatch, SearchQueryMatch, SearchQuerySubmatch};
        use std::path::PathBuf;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Response::SearchResults {
                id: SearchId::MAX,
                matches: vec![SearchQueryMatch::Contents(SearchQueryContentsMatch {
                    path: PathBuf::from("path"),
                    lines: "some lines".into(),
                    line_number: u64::MAX,
                    absolute_offset: u64::MAX,
                    submatches: vec![SearchQuerySubmatch::new("text", u64::MAX, u64::MAX)],
                })],
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "search_results",
                    "id": SearchId::MAX,
                    "matches": [{
                        "type": "contents",
                        "path": "path",
                        "lines": "some lines",
                        "line_number": u64::MAX,
                        "absolute_offset": u64::MAX,
                        "submatches": [{
                            "match": "text",
                            "start": u64::MAX,
                            "end": u64::MAX,
                        }],
                    }],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "search_results",
                "id": SearchId::MAX,
                "matches": [{
                    "type": "contents",
                    "path": "path",
                    "lines": "some lines",
                    "line_number": u64::MAX,
                    "absolute_offset": u64::MAX,
                    "submatches": [{
                        "match": "text",
                        "start": u64::MAX,
                        "end": u64::MAX,
                    }],
                }],
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Response::SearchResults {
                    id: SearchId::MAX,
                    matches: vec![SearchQueryMatch::Contents(SearchQueryContentsMatch {
                        path: PathBuf::from("path"),
                        lines: "some lines".into(),
                        line_number: u64::MAX,
                        absolute_offset: u64::MAX,
                        submatches: vec![SearchQuerySubmatch::new("text", u64::MAX, u64::MAX)],
                    })],
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Response::SearchResults {
                id: SearchId::MAX,
                matches: vec![SearchQueryMatch::Contents(SearchQueryContentsMatch {
                    path: PathBuf::from("path"),
                    lines: "some lines".into(),
                    line_number: u64::MAX,
                    absolute_offset: u64::MAX,
                    submatches: vec![SearchQuerySubmatch::new("text", u64::MAX, u64::MAX)],
                })],
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is results. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::SearchResults {
                id: SearchId::MAX,
                matches: vec![SearchQueryMatch::Contents(SearchQueryContentsMatch {
                    path: PathBuf::from("path"),
                    lines: "some lines".into(),
                    line_number: u64::MAX,
                    absolute_offset: u64::MAX,
                    submatches: vec![SearchQuerySubmatch::new("text", u64::MAX, u64::MAX)],
                })],
            })
            .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Response::SearchResults {
                    id: SearchId::MAX,
                    matches: vec![SearchQueryMatch::Contents(SearchQueryContentsMatch {
                        path: PathBuf::from("path"),
                        lines: "some lines".into(),
                        line_number: u64::MAX,
                        absolute_offset: u64::MAX,
                        submatches: vec![SearchQuerySubmatch::new("text", u64::MAX, u64::MAX)],
                    })],
                }
            );
        }
    }

    mod search_done {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Response::SearchDone { id: SearchId::MAX };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "search_done",
                    "id": SearchId::MAX,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "search_done",
                "id": SearchId::MAX,
            });

            let payload: Response = serde_json::from_value(value).unwrap();
            assert_eq!(payload, Response::SearchDone { id: SearchId::MAX });
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Response::SearchDone { id: SearchId::MAX };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Response::SearchDone { id: SearchId::MAX })
                .unwrap();

            let payload: Response = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(payload, Response::SearchDone { id: SearchId::MAX });
        }
    }

    mod proc_spawned {
        use super::*;
    }

    mod proc_stdout {
        use super::*;
    }

    mod proc_stderr {
        use super::*;
    }

    mod proc_done {
        use super::*;
    }

    mod system_info {
        use super::*;
    }

    mod capabilities {
        use super::*;
    }
}
