use std::collections::HashMap;
use std::path::PathBuf;

use derive_more::IsVariant;
use serde::{Deserialize, Serialize};
use strum::{AsRefStr, EnumDiscriminants, EnumIter, EnumMessage, EnumString};

use crate::common::{
    ChangeKind, Cmd, Permissions, ProcessId, PtySize, SearchId, SearchQuery, SetPermissionsOptions,
};
use crate::utils;

/// Mapping of environment variables
pub type Environment = HashMap<String, String>;

/// Represents the payload of a request to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, EnumDiscriminants, IsVariant, Serialize, Deserialize)]
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
#[strum_discriminants(name(RequestKind))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum Request {
    /// Reads a file from the specified path on the remote machine
    #[strum_discriminants(strum(message = "Supports reading binary file"))]
    FileRead {
        /// The path to the file on the remote machine
        path: PathBuf,
    },

    /// Reads a file from the specified path on the remote machine
    /// and treats the contents as text
    #[strum_discriminants(strum(message = "Supports reading text file"))]
    FileReadText {
        /// The path to the file on the remote machine
        path: PathBuf,
    },

    /// Writes a file, creating it if it does not exist, and overwriting any existing content
    /// on the remote machine
    #[strum_discriminants(strum(message = "Supports writing binary file"))]
    FileWrite {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },

    /// Writes a file using text instead of bytes, creating it if it does not exist,
    /// and overwriting any existing content on the remote machine
    #[strum_discriminants(strum(message = "Supports writing text file"))]
    FileWriteText {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content
        text: String,
    },

    /// Appends to a file, creating it if it does not exist, on the remote machine
    #[strum_discriminants(strum(message = "Supports appending to binary file"))]
    FileAppend {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },

    /// Appends text to a file, creating it if it does not exist, on the remote machine
    #[strum_discriminants(strum(message = "Supports appending to text file"))]
    FileAppendText {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content
        text: String,
    },

    /// Reads a directory from the specified path on the remote machine
    #[strum_discriminants(strum(message = "Supports reading directory"))]
    DirRead {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Maximum depth to traverse with 0 indicating there is no maximum
        /// depth and 1 indicating the most immediate children within the
        /// directory
        #[serde(default = "utils::one", skip_serializing_if = "utils::is_one")]
        depth: usize,

        /// Whether or not to return absolute or relative paths
        #[serde(default, skip_serializing_if = "utils::is_false")]
        absolute: bool,

        /// Whether or not to canonicalize the resulting paths, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        ///
        /// Note that the flag absolute must be true to have absolute paths
        /// returned, even if canonicalize is flagged as true
        #[serde(default, skip_serializing_if = "utils::is_false")]
        canonicalize: bool,

        /// Whether or not to include the root directory in the retrieved
        /// entries
        ///
        /// If included, the root directory will also be a canonicalized,
        /// absolute path and will not follow any of the other flags
        #[serde(default, skip_serializing_if = "utils::is_false")]
        include_root: bool,
    },

    /// Creates a directory on the remote machine
    #[strum_discriminants(strum(message = "Supports creating directory"))]
    DirCreate {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Whether or not to create all parent directories
        #[serde(default, skip_serializing_if = "utils::is_false")]
        all: bool,
    },

    /// Removes a file or directory on the remote machine
    #[strum_discriminants(strum(message = "Supports removing files, directories, and symlinks"))]
    Remove {
        /// The path to the file or directory on the remote machine
        path: PathBuf,

        /// Whether or not to remove all contents within directory if is a directory.
        /// Does nothing different for files
        #[serde(default, skip_serializing_if = "utils::is_false")]
        force: bool,
    },

    /// Copies a file or directory on the remote machine
    #[strum_discriminants(strum(message = "Supports copying files, directories, and symlinks"))]
    Copy {
        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for copy of file or directory
        dst: PathBuf,
    },

    /// Moves/renames a file or directory on the remote machine
    #[strum_discriminants(strum(message = "Supports renaming files, directories, and symlinks"))]
    Rename {
        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for the file or directory
        dst: PathBuf,
    },

    /// Watches a path for changes
    #[strum_discriminants(strum(message = "Supports watching filesystem for changes"))]
    Watch {
        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,

        /// If true, will recursively watch for changes within directories, othewise
        /// will only watch for changes immediately within directories
        #[serde(default, skip_serializing_if = "utils::is_false")]
        recursive: bool,

        /// Filter to only report back specified changes
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        only: Vec<ChangeKind>,

        /// Filter to report back changes except these specified changes
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        except: Vec<ChangeKind>,
    },

    /// Unwatches a path for changes, meaning no additional changes will be reported
    #[strum_discriminants(strum(message = "Supports unwatching filesystem for changes"))]
    Unwatch {
        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,
    },

    /// Checks whether the given path exists
    #[strum_discriminants(strum(message = "Supports checking if a path exists"))]
    Exists {
        /// The path to the file or directory on the remote machine
        path: PathBuf,
    },

    /// Retrieves filesystem metadata for the specified path on the remote machine
    #[strum_discriminants(strum(
        message = "Supports retrieving metadata about a file, directory, or symlink"
    ))]
    Metadata {
        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,

        /// Whether or not to include a canonicalized version of the path, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        #[serde(default, skip_serializing_if = "utils::is_false")]
        canonicalize: bool,

        /// Whether or not to follow symlinks to determine absolute file type (dir/file)
        #[serde(default, skip_serializing_if = "utils::is_false")]
        resolve_file_type: bool,
    },

    /// Sets permissions on a file, directory, or symlink on the remote machine
    #[strum_discriminants(strum(
        message = "Supports setting permissions on a file, directory, or symlink"
    ))]
    SetPermissions {
        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,

        /// New permissions to apply to the file, directory, or symlink
        permissions: Permissions,

        /// Additional options to supply when setting permissions
        #[serde(default)]
        options: SetPermissionsOptions,
    },

    /// Searches filesystem using the provided query
    #[strum_discriminants(strum(message = "Supports searching filesystem using queries"))]
    Search {
        /// Query to perform against the filesystem
        query: SearchQuery,
    },

    /// Cancels an active search being run against the filesystem
    #[strum_discriminants(strum(
        message = "Supports canceling an active search against the filesystem"
    ))]
    CancelSearch {
        /// Id of the search to cancel
        id: SearchId,
    },

    /// Spawns a new process on the remote machine
    #[strum_discriminants(strum(message = "Supports spawning a process"))]
    ProcSpawn {
        /// The full command to run including arguments
        cmd: Cmd,

        /// Environment to provide to the remote process
        #[serde(default, skip_serializing_if = "HashMap::is_empty")]
        environment: Environment,

        /// Alternative current directory for the remote process
        #[serde(default, skip_serializing_if = "Option::is_none")]
        current_dir: Option<PathBuf>,

        /// If provided, will spawn process in a pty, otherwise spawns directly
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pty: Option<PtySize>,
    },

    /// Kills a process running on the remote machine
    #[strum_discriminants(strum(message = "Supports killing a spawned process"))]
    ProcKill {
        /// Id of the actively-running process
        id: ProcessId,
    },

    /// Sends additional data to stdin of running process
    #[strum_discriminants(strum(message = "Supports sending stdin to a spawned process"))]
    ProcStdin {
        /// Id of the actively-running process to send stdin data
        id: ProcessId,

        /// Data to send to a process's stdin pipe
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },

    /// Resize pty of remote process
    #[strum_discriminants(strum(message = "Supports resizing the pty of a spawned process"))]
    ProcResizePty {
        /// Id of the actively-running process whose pty to resize
        id: ProcessId,

        /// The new pty dimensions
        size: PtySize,
    },

    /// Retrieve information about the server and the system it is on
    #[strum_discriminants(strum(message = "Supports retrieving system information"))]
    SystemInfo {},

    /// Retrieve information about the server's protocol version
    #[strum_discriminants(strum(message = "Supports retrieving version"))]
    Version {},
}

#[cfg(test)]
mod tests {
    use super::*;

    mod file_read {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::FileRead {
                path: PathBuf::from("path"),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "file_read",
                    "path": "path",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "file_read",
                "path": "path",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::FileRead {
                    path: PathBuf::from("path"),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::FileRead {
                path: PathBuf::from("path"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::FileRead {
                path: PathBuf::from("path"),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::FileRead {
                    path: PathBuf::from("path"),
                }
            );
        }
    }

    mod file_read_text {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::FileReadText {
                path: PathBuf::from("path"),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "file_read_text",
                    "path": "path",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "file_read_text",
                "path": "path",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::FileReadText {
                    path: PathBuf::from("path"),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::FileReadText {
                path: PathBuf::from("path"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::FileReadText {
                path: PathBuf::from("path"),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::FileReadText {
                    path: PathBuf::from("path"),
                }
            );
        }
    }

    mod file_write {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::FileWrite {
                path: PathBuf::from("path"),
                data: vec![0, 1, 2, u8::MAX],
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "file_write",
                    "path": "path",
                    "data": [0, 1, 2, u8::MAX],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "file_write",
                "path": "path",
                "data": [0, 1, 2, u8::MAX],
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::FileWrite {
                    path: PathBuf::from("path"),
                    data: vec![0, 1, 2, u8::MAX],
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::FileWrite {
                path: PathBuf::from("path"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::FileWrite {
                path: PathBuf::from("path"),
                data: vec![0, 1, 2, u8::MAX],
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::FileWrite {
                    path: PathBuf::from("path"),
                    data: vec![0, 1, 2, u8::MAX],
                }
            );
        }
    }

    mod file_write_text {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::FileWriteText {
                path: PathBuf::from("path"),
                text: String::from("text"),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "file_write_text",
                    "path": "path",
                    "text": "text",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "file_write_text",
                "path": "path",
                "text": "text",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::FileWriteText {
                    path: PathBuf::from("path"),
                    text: String::from("text"),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::FileWriteText {
                path: PathBuf::from("path"),
                text: String::from("text"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::FileWriteText {
                path: PathBuf::from("path"),
                text: String::from("text"),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::FileWriteText {
                    path: PathBuf::from("path"),
                    text: String::from("text"),
                }
            );
        }
    }

    mod file_append {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::FileAppend {
                path: PathBuf::from("path"),
                data: vec![0, 1, 2, u8::MAX],
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "file_append",
                    "path": "path",
                    "data": [0, 1, 2, u8::MAX],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "file_append",
                "path": "path",
                "data": [0, 1, 2, u8::MAX],
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::FileAppend {
                    path: PathBuf::from("path"),
                    data: vec![0, 1, 2, u8::MAX],
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::FileAppend {
                path: PathBuf::from("path"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::FileAppend {
                path: PathBuf::from("path"),
                data: vec![0, 1, 2, u8::MAX],
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::FileAppend {
                    path: PathBuf::from("path"),
                    data: vec![0, 1, 2, u8::MAX],
                }
            );
        }
    }

    mod file_append_text {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::FileAppendText {
                path: PathBuf::from("path"),
                text: String::from("text"),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "file_append_text",
                    "path": "path",
                    "text": "text",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "file_append_text",
                "path": "path",
                "text": "text",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::FileAppendText {
                    path: PathBuf::from("path"),
                    text: String::from("text"),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::FileAppendText {
                path: PathBuf::from("path"),
                text: String::from("text"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::FileAppendText {
                path: PathBuf::from("path"),
                text: String::from("text"),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::FileAppendText {
                    path: PathBuf::from("path"),
                    text: String::from("text"),
                }
            );
        }
    }

    mod dir_read {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Request::DirRead {
                path: PathBuf::from("path"),
                depth: 1,
                absolute: false,
                canonicalize: false,
                include_root: false,
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "dir_read",
                    "path": "path",
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Request::DirRead {
                path: PathBuf::from("path"),
                depth: usize::MAX,
                absolute: true,
                canonicalize: true,
                include_root: true,
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "dir_read",
                    "path": "path",
                    "depth": usize::MAX,
                    "absolute": true,
                    "canonicalize": true,
                    "include_root": true,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "dir_read",
                "path": "path",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::DirRead {
                    path: PathBuf::from("path"),
                    depth: 1,
                    absolute: false,
                    canonicalize: false,
                    include_root: false,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let value = serde_json::json!({
                "type": "dir_read",
                "path": "path",
                "depth": usize::MAX,
                "absolute": true,
                "canonicalize": true,
                "include_root": true,
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::DirRead {
                    path: PathBuf::from("path"),
                    depth: usize::MAX,
                    absolute: true,
                    canonicalize: true,
                    include_root: true,
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Request::DirRead {
                path: PathBuf::from("path"),
                depth: 1,
                absolute: false,
                canonicalize: false,
                include_root: false,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Request::DirRead {
                path: PathBuf::from("path"),
                depth: usize::MAX,
                absolute: true,
                canonicalize: true,
                include_root: true,
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
            let buf = rmp_serde::encode::to_vec_named(&Request::DirRead {
                path: PathBuf::from("path"),
                depth: 1,
                absolute: false,
                canonicalize: false,
                include_root: false,
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::DirRead {
                    path: PathBuf::from("path"),
                    depth: 1,
                    absolute: false,
                    canonicalize: false,
                    include_root: false,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Request::DirRead {
                path: PathBuf::from("path"),
                depth: usize::MAX,
                absolute: true,
                canonicalize: true,
                include_root: true,
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::DirRead {
                    path: PathBuf::from("path"),
                    depth: usize::MAX,
                    absolute: true,
                    canonicalize: true,
                    include_root: true,
                }
            );
        }
    }

    mod dir_create {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Request::DirCreate {
                path: PathBuf::from("path"),
                all: false,
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "dir_create",
                    "path": "path",
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Request::DirCreate {
                path: PathBuf::from("path"),
                all: true,
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "dir_create",
                    "path": "path",
                    "all": true,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "dir_create",
                "path": "path",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::DirCreate {
                    path: PathBuf::from("path"),
                    all: false,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let value = serde_json::json!({
                "type": "dir_create",
                "path": "path",
                "all": true,
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::DirCreate {
                    path: PathBuf::from("path"),
                    all: true,
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Request::DirCreate {
                path: PathBuf::from("path"),
                all: false,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Request::DirCreate {
                path: PathBuf::from("path"),
                all: true,
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
            let buf = rmp_serde::encode::to_vec_named(&Request::DirCreate {
                path: PathBuf::from("path"),
                all: false,
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::DirCreate {
                    path: PathBuf::from("path"),
                    all: false,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Request::DirCreate {
                path: PathBuf::from("path"),
                all: true,
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::DirCreate {
                    path: PathBuf::from("path"),
                    all: true,
                }
            );
        }
    }

    mod remove {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Request::Remove {
                path: PathBuf::from("path"),
                force: false,
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "remove",
                    "path": "path",
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Request::Remove {
                path: PathBuf::from("path"),
                force: true,
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "remove",
                    "path": "path",
                    "force": true,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "remove",
                "path": "path",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Remove {
                    path: PathBuf::from("path"),
                    force: false,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let value = serde_json::json!({
                "type": "remove",
                "path": "path",
                "force": true,
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Remove {
                    path: PathBuf::from("path"),
                    force: true,
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Request::Remove {
                path: PathBuf::from("path"),
                force: false,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Request::Remove {
                path: PathBuf::from("path"),
                force: true,
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
            let buf = rmp_serde::encode::to_vec_named(&Request::Remove {
                path: PathBuf::from("path"),
                force: false,
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Remove {
                    path: PathBuf::from("path"),
                    force: false,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Request::Remove {
                path: PathBuf::from("path"),
                force: true,
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Remove {
                    path: PathBuf::from("path"),
                    force: true,
                }
            );
        }
    }

    mod copy {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::Copy {
                src: PathBuf::from("src"),
                dst: PathBuf::from("dst"),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "copy",
                    "src": "src",
                    "dst": "dst",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "copy",
                "src": "src",
                "dst": "dst",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Copy {
                    src: PathBuf::from("src"),
                    dst: PathBuf::from("dst"),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::Copy {
                src: PathBuf::from("src"),
                dst: PathBuf::from("dst"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::Copy {
                src: PathBuf::from("src"),
                dst: PathBuf::from("dst"),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Copy {
                    src: PathBuf::from("src"),
                    dst: PathBuf::from("dst"),
                }
            );
        }
    }

    mod rename {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::Rename {
                src: PathBuf::from("src"),
                dst: PathBuf::from("dst"),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "rename",
                    "src": "src",
                    "dst": "dst",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "rename",
                "src": "src",
                "dst": "dst",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Rename {
                    src: PathBuf::from("src"),
                    dst: PathBuf::from("dst"),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::Rename {
                src: PathBuf::from("src"),
                dst: PathBuf::from("dst"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::Rename {
                src: PathBuf::from("src"),
                dst: PathBuf::from("dst"),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Rename {
                    src: PathBuf::from("src"),
                    dst: PathBuf::from("dst"),
                }
            );
        }
    }

    mod watch {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Request::Watch {
                path: PathBuf::from("path"),
                recursive: false,
                only: Vec::new(),
                except: Vec::new(),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "watch",
                    "path": "path",
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Request::Watch {
                path: PathBuf::from("path"),
                recursive: true,
                only: vec![ChangeKind::Access],
                except: vec![ChangeKind::Modify],
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "watch",
                    "path": "path",
                    "recursive": true,
                    "only": ["access"],
                    "except": ["modify"],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "watch",
                "path": "path",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Watch {
                    path: PathBuf::from("path"),
                    recursive: false,
                    only: Vec::new(),
                    except: Vec::new(),
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let value = serde_json::json!({
                "type": "watch",
                "path": "path",
                "recursive": true,
                "only": ["access"],
                "except": ["modify"],
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Watch {
                    path: PathBuf::from("path"),
                    recursive: true,
                    only: vec![ChangeKind::Access],
                    except: vec![ChangeKind::Modify],
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Request::Watch {
                path: PathBuf::from("path"),
                recursive: false,
                only: Vec::new(),
                except: Vec::new(),
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Request::Watch {
                path: PathBuf::from("path"),
                recursive: true,
                only: vec![ChangeKind::Access],
                except: vec![ChangeKind::Modify],
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
            let buf = rmp_serde::encode::to_vec_named(&Request::Watch {
                path: PathBuf::from("path"),
                recursive: false,
                only: Vec::new(),
                except: Vec::new(),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Watch {
                    path: PathBuf::from("path"),
                    recursive: false,
                    only: Vec::new(),
                    except: Vec::new(),
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Request::Watch {
                path: PathBuf::from("path"),
                recursive: true,
                only: vec![ChangeKind::Access],
                except: vec![ChangeKind::Modify],
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Watch {
                    path: PathBuf::from("path"),
                    recursive: true,
                    only: vec![ChangeKind::Access],
                    except: vec![ChangeKind::Modify],
                }
            );
        }
    }

    mod unwatch {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::Unwatch {
                path: PathBuf::from("path"),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "unwatch",
                    "path": "path",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "unwatch",
                "path": "path",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Unwatch {
                    path: PathBuf::from("path")
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::Unwatch {
                path: PathBuf::from("path"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::Unwatch {
                path: PathBuf::from("path"),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Unwatch {
                    path: PathBuf::from("path"),
                }
            );
        }
    }

    mod exists {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::Exists {
                path: PathBuf::from("path"),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "exists",
                    "path": "path",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "exists",
                "path": "path",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Exists {
                    path: PathBuf::from("path")
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::Exists {
                path: PathBuf::from("path"),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::Exists {
                path: PathBuf::from("path"),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Exists {
                    path: PathBuf::from("path"),
                }
            );
        }
    }

    mod metadata {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Request::Metadata {
                path: PathBuf::from("path"),
                canonicalize: false,
                resolve_file_type: false,
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "metadata",
                    "path": "path",
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Request::Metadata {
                path: PathBuf::from("path"),
                canonicalize: true,
                resolve_file_type: true,
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "metadata",
                    "path": "path",
                    "canonicalize": true,
                    "resolve_file_type": true,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "metadata",
                "path": "path",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Metadata {
                    path: PathBuf::from("path"),
                    canonicalize: false,
                    resolve_file_type: false,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let value = serde_json::json!({
                "type": "metadata",
                "path": "path",
                "canonicalize": true,
                "resolve_file_type": true,
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Metadata {
                    path: PathBuf::from("path"),
                    canonicalize: true,
                    resolve_file_type: true,
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Request::Metadata {
                path: PathBuf::from("path"),
                canonicalize: false,
                resolve_file_type: false,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Request::Metadata {
                path: PathBuf::from("path"),
                canonicalize: true,
                resolve_file_type: true,
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
            let buf = rmp_serde::encode::to_vec_named(&Request::Metadata {
                path: PathBuf::from("path"),
                canonicalize: false,
                resolve_file_type: false,
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Metadata {
                    path: PathBuf::from("path"),
                    canonicalize: false,
                    resolve_file_type: false,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Request::Metadata {
                path: PathBuf::from("path"),
                canonicalize: true,
                resolve_file_type: true,
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Metadata {
                    path: PathBuf::from("path"),
                    canonicalize: true,
                    resolve_file_type: true,
                }
            );
        }
    }

    mod set_permissions {
        use super::*;

        const fn full_permissions() -> Permissions {
            Permissions {
                owner_read: Some(true),
                owner_write: Some(true),
                owner_exec: Some(true),
                group_read: Some(true),
                group_write: Some(true),
                group_exec: Some(true),
                other_read: Some(true),
                other_write: Some(true),
                other_exec: Some(true),
            }
        }

        const fn full_options() -> SetPermissionsOptions {
            SetPermissionsOptions {
                exclude_symlinks: true,
                follow_symlinks: true,
                recursive: true,
            }
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Request::SetPermissions {
                path: PathBuf::from("path"),
                permissions: Default::default(),
                options: Default::default(),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "set_permissions",
                    "path": "path",
                    "permissions": {},
                    "options": {},
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Request::SetPermissions {
                path: PathBuf::from("path"),
                permissions: full_permissions(),
                options: full_options(),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "set_permissions",
                    "path": "path",
                    "permissions": {
                        "owner_read": true,
                        "owner_write": true,
                        "owner_exec": true,
                        "group_read": true,
                        "group_write": true,
                        "group_exec": true,
                        "other_read": true,
                        "other_write": true,
                        "other_exec": true,
                    },
                    "options": {
                        "exclude_symlinks": true,
                        "follow_symlinks": true,
                        "recursive": true,
                    },
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "set_permissions",
                "path": "path",
                "permissions": {},
                "options": {},
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::SetPermissions {
                    path: PathBuf::from("path"),
                    permissions: Default::default(),
                    options: Default::default(),
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let value = serde_json::json!({
                "type": "set_permissions",
                "path": "path",
                "permissions": {
                    "owner_read": true,
                    "owner_write": true,
                    "owner_exec": true,
                    "group_read": true,
                    "group_write": true,
                    "group_exec": true,
                    "other_read": true,
                    "other_write": true,
                    "other_exec": true,
                },
                "options": {
                    "exclude_symlinks": true,
                    "follow_symlinks": true,
                    "recursive": true,
                },
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::SetPermissions {
                    path: PathBuf::from("path"),
                    permissions: full_permissions(),
                    options: full_options(),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Request::SetPermissions {
                path: PathBuf::from("path"),
                permissions: Default::default(),
                options: Default::default(),
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Request::SetPermissions {
                path: PathBuf::from("path"),
                permissions: full_permissions(),
                options: full_options(),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::SetPermissions {
                path: PathBuf::from("path"),
                permissions: Default::default(),
                options: Default::default(),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::SetPermissions {
                    path: PathBuf::from("path"),
                    permissions: Default::default(),
                    options: Default::default(),
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Request::SetPermissions {
                path: PathBuf::from("path"),
                permissions: full_permissions(),
                options: full_options(),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::SetPermissions {
                    path: PathBuf::from("path"),
                    permissions: full_permissions(),
                    options: full_options(),
                }
            );
        }
    }

    mod search {
        use super::*;
        use crate::common::{
            FileType, SearchQueryCondition, SearchQueryOptions, SearchQueryTarget,
        };

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Request::Search {
                query: SearchQuery {
                    target: SearchQueryTarget::Contents,
                    condition: SearchQueryCondition::equals("hello world"),
                    paths: vec![PathBuf::from("path")],
                    options: Default::default(),
                },
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "search",
                    "query": {
                        "target": "contents",
                        "condition": {
                            "type": "equals",
                            "value": "hello world",
                        },
                        "paths": ["path"],
                        "options": {},
                    },
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Request::Search {
                query: SearchQuery {
                    target: SearchQueryTarget::Contents,
                    condition: SearchQueryCondition::equals("hello world"),
                    paths: vec![PathBuf::from("path")],
                    options: SearchQueryOptions {
                        allowed_file_types: [FileType::File].into_iter().collect(),
                        include: Some(SearchQueryCondition::Equals {
                            value: String::from("hello"),
                        }),
                        exclude: Some(SearchQueryCondition::Contains {
                            value: String::from("world"),
                        }),
                        upward: true,
                        follow_symbolic_links: true,
                        limit: Some(u64::MAX),
                        max_depth: Some(u64::MAX),
                        pagination: Some(u64::MAX),
                        ignore_hidden: true,
                        use_ignore_files: true,
                        use_parent_ignore_files: true,
                        use_git_ignore_files: true,
                        use_global_git_ignore_files: true,
                        use_git_exclude_files: true,
                    },
                },
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "search",
                    "query": {
                        "target": "contents",
                        "condition": {
                            "type": "equals",
                            "value": "hello world",
                        },
                        "paths": ["path"],
                        "options": {
                            "allowed_file_types": ["file"],
                            "include": {
                                "type": "equals",
                                "value": "hello",
                            },
                            "exclude": {
                                "type": "contains",
                                "value": "world",
                            },
                            "upward": true,
                            "follow_symbolic_links": true,
                            "limit": u64::MAX,
                            "max_depth": u64::MAX,
                            "pagination": u64::MAX,
                            "ignore_hidden": true,
                            "use_ignore_files": true,
                            "use_parent_ignore_files": true,
                            "use_git_ignore_files": true,
                            "use_global_git_ignore_files": true,
                            "use_git_exclude_files": true,
                        },
                    },
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "search",
                "query": {
                    "target": "contents",
                    "condition": {
                        "type": "equals",
                        "value": "hello world",
                    },
                    "paths": ["path"],
                },
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Search {
                    query: SearchQuery {
                        target: SearchQueryTarget::Contents,
                        condition: SearchQueryCondition::equals("hello world"),
                        paths: vec![PathBuf::from("path")],
                        options: Default::default(),
                    },
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let value = serde_json::json!({
                "type": "search",
                "query": {
                    "target": "contents",
                    "condition": {
                        "type": "equals",
                        "value": "hello world",
                    },
                    "paths": ["path"],
                    "options": {
                        "allowed_file_types": ["file"],
                        "include": {
                            "type": "equals",
                            "value": "hello",
                        },
                        "exclude": {
                            "type": "contains",
                            "value": "world",
                        },
                        "upward": true,
                        "follow_symbolic_links": true,
                        "limit": u64::MAX,
                        "max_depth": u64::MAX,
                        "pagination": u64::MAX,
                        "ignore_hidden": true,
                        "use_ignore_files": true,
                        "use_parent_ignore_files": true,
                        "use_git_ignore_files": true,
                        "use_global_git_ignore_files": true,
                        "use_git_exclude_files": true,
                    },
                },
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::Search {
                    query: SearchQuery {
                        target: SearchQueryTarget::Contents,
                        condition: SearchQueryCondition::equals("hello world"),
                        paths: vec![PathBuf::from("path")],
                        options: SearchQueryOptions {
                            allowed_file_types: [FileType::File].into_iter().collect(),
                            include: Some(SearchQueryCondition::Equals {
                                value: String::from("hello"),
                            }),
                            exclude: Some(SearchQueryCondition::Contains {
                                value: String::from("world"),
                            }),
                            upward: true,
                            follow_symbolic_links: true,
                            limit: Some(u64::MAX),
                            max_depth: Some(u64::MAX),
                            pagination: Some(u64::MAX),
                            ignore_hidden: true,
                            use_ignore_files: true,
                            use_parent_ignore_files: true,
                            use_git_ignore_files: true,
                            use_global_git_ignore_files: true,
                            use_git_exclude_files: true,
                        },
                    },
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Request::Search {
                query: SearchQuery {
                    target: SearchQueryTarget::Contents,
                    condition: SearchQueryCondition::equals("hello world"),
                    paths: vec![PathBuf::from("path")],
                    options: Default::default(),
                },
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Request::Search {
                query: SearchQuery {
                    target: SearchQueryTarget::Contents,
                    condition: SearchQueryCondition::equals("hello world"),
                    paths: vec![PathBuf::from("path")],
                    options: SearchQueryOptions {
                        allowed_file_types: [FileType::File].into_iter().collect(),
                        include: Some(SearchQueryCondition::Equals {
                            value: String::from("hello"),
                        }),
                        exclude: Some(SearchQueryCondition::Contains {
                            value: String::from("world"),
                        }),
                        upward: true,
                        follow_symbolic_links: true,
                        limit: Some(u64::MAX),
                        max_depth: Some(u64::MAX),
                        pagination: Some(u64::MAX),
                        ignore_hidden: true,
                        use_ignore_files: true,
                        use_parent_ignore_files: true,
                        use_git_ignore_files: true,
                        use_global_git_ignore_files: true,
                        use_git_exclude_files: true,
                    },
                },
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
            let buf = rmp_serde::encode::to_vec_named(&Request::Search {
                query: SearchQuery {
                    target: SearchQueryTarget::Contents,
                    condition: SearchQueryCondition::equals("hello world"),
                    paths: vec![PathBuf::from("path")],
                    options: Default::default(),
                },
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Search {
                    query: SearchQuery {
                        target: SearchQueryTarget::Contents,
                        condition: SearchQueryCondition::equals("hello world"),
                        paths: vec![PathBuf::from("path")],
                        options: Default::default(),
                    },
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Request::Search {
                query: SearchQuery {
                    target: SearchQueryTarget::Contents,
                    condition: SearchQueryCondition::equals("hello world"),
                    paths: vec![PathBuf::from("path")],
                    options: SearchQueryOptions {
                        allowed_file_types: [FileType::File].into_iter().collect(),
                        include: Some(SearchQueryCondition::Equals {
                            value: String::from("hello"),
                        }),
                        exclude: Some(SearchQueryCondition::Contains {
                            value: String::from("world"),
                        }),
                        upward: true,
                        follow_symbolic_links: true,
                        limit: Some(u64::MAX),
                        max_depth: Some(u64::MAX),
                        pagination: Some(u64::MAX),
                        ignore_hidden: true,
                        use_ignore_files: true,
                        use_parent_ignore_files: true,
                        use_git_ignore_files: true,
                        use_global_git_ignore_files: true,
                        use_git_exclude_files: true,
                    },
                },
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::Search {
                    query: SearchQuery {
                        target: SearchQueryTarget::Contents,
                        condition: SearchQueryCondition::equals("hello world"),
                        paths: vec![PathBuf::from("path")],
                        options: SearchQueryOptions {
                            allowed_file_types: [FileType::File].into_iter().collect(),
                            include: Some(SearchQueryCondition::Equals {
                                value: String::from("hello"),
                            }),
                            exclude: Some(SearchQueryCondition::Contains {
                                value: String::from("world"),
                            }),
                            upward: true,
                            follow_symbolic_links: true,
                            limit: Some(u64::MAX),
                            max_depth: Some(u64::MAX),
                            pagination: Some(u64::MAX),
                            ignore_hidden: true,
                            use_ignore_files: true,
                            use_parent_ignore_files: true,
                            use_git_ignore_files: true,
                            use_global_git_ignore_files: true,
                            use_git_exclude_files: true,
                        },
                    },
                }
            );
        }
    }

    mod cancel_search {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::CancelSearch { id: u32::MAX };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "cancel_search",
                    "id": u32::MAX,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "cancel_search",
                "id": u32::MAX,
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(payload, Request::CancelSearch { id: u32::MAX });
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::CancelSearch { id: u32::MAX };

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
                rmp_serde::encode::to_vec_named(&Request::CancelSearch { id: u32::MAX }).unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(payload, Request::CancelSearch { id: u32::MAX });
        }
    }

    mod proc_spawn {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_json() {
            let payload = Request::ProcSpawn {
                cmd: Cmd::new("echo some text"),
                environment: Environment::new(),
                current_dir: None,
                pty: None,
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "proc_spawn",
                    "cmd": "echo some text",
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_json() {
            let payload = Request::ProcSpawn {
                cmd: Cmd::new("echo some text"),
                environment: [(String::from("hello"), String::from("world"))]
                    .into_iter()
                    .collect(),
                current_dir: Some(PathBuf::from("current-dir")),
                pty: Some(PtySize {
                    rows: u16::MAX,
                    cols: u16::MAX,
                    pixel_width: u16::MAX,
                    pixel_height: u16::MAX,
                }),
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "proc_spawn",
                    "cmd": "echo some text",
                    "environment": { "hello": "world" },
                    "current_dir": "current-dir",
                    "pty": {
                        "rows": u16::MAX,
                        "cols": u16::MAX,
                        "pixel_width": u16::MAX,
                        "pixel_height": u16::MAX,
                    },
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_payload_from_json() {
            let value = serde_json::json!({
                "type": "proc_spawn",
                "cmd": "echo some text",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::ProcSpawn {
                    cmd: Cmd::new("echo some text"),
                    environment: Environment::new(),
                    current_dir: None,
                    pty: None,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_json() {
            let value = serde_json::json!({
                "type": "proc_spawn",
                "cmd": "echo some text",
                "environment": { "hello": "world" },
                "current_dir": "current-dir",
                "pty": {
                    "rows": u16::MAX,
                    "cols": u16::MAX,
                    "pixel_width": u16::MAX,
                    "pixel_height": u16::MAX,
                },
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::ProcSpawn {
                    cmd: Cmd::new("echo some text"),
                    environment: [(String::from("hello"), String::from("world"))]
                        .into_iter()
                        .collect(),
                    current_dir: Some(PathBuf::from("current-dir")),
                    pty: Some(PtySize {
                        rows: u16::MAX,
                        cols: u16::MAX,
                        pixel_width: u16::MAX,
                        pixel_height: u16::MAX,
                    }),
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_payload_to_msgpack() {
            let payload = Request::ProcSpawn {
                cmd: Cmd::new("echo some text"),
                environment: Environment::new(),
                current_dir: None,
                pty: None,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&payload).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_payload_to_msgpack() {
            let payload = Request::ProcSpawn {
                cmd: Cmd::new("echo some text"),
                environment: [(String::from("hello"), String::from("world"))]
                    .into_iter()
                    .collect(),
                current_dir: Some(PathBuf::from("current-dir")),
                pty: Some(PtySize {
                    rows: u16::MAX,
                    cols: u16::MAX,
                    pixel_width: u16::MAX,
                    pixel_height: u16::MAX,
                }),
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
            let buf = rmp_serde::encode::to_vec_named(&Request::ProcSpawn {
                cmd: Cmd::new("echo some text"),
                environment: Environment::new(),
                current_dir: None,
                pty: None,
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::ProcSpawn {
                    cmd: Cmd::new("echo some text"),
                    environment: Environment::new(),
                    current_dir: None,
                    pty: None,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_payload_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Request::ProcSpawn {
                cmd: Cmd::new("echo some text"),
                environment: [(String::from("hello"), String::from("world"))]
                    .into_iter()
                    .collect(),
                current_dir: Some(PathBuf::from("current-dir")),
                pty: Some(PtySize {
                    rows: u16::MAX,
                    cols: u16::MAX,
                    pixel_width: u16::MAX,
                    pixel_height: u16::MAX,
                }),
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::ProcSpawn {
                    cmd: Cmd::new("echo some text"),
                    environment: [(String::from("hello"), String::from("world"))]
                        .into_iter()
                        .collect(),
                    current_dir: Some(PathBuf::from("current-dir")),
                    pty: Some(PtySize {
                        rows: u16::MAX,
                        cols: u16::MAX,
                        pixel_width: u16::MAX,
                        pixel_height: u16::MAX,
                    }),
                }
            );
        }
    }

    mod proc_kill {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::ProcKill { id: u32::MAX };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "proc_kill",
                    "id": u32::MAX,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "proc_kill",
                "id": u32::MAX,
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(payload, Request::ProcKill { id: u32::MAX });
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::ProcKill { id: u32::MAX };

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
            let buf = rmp_serde::encode::to_vec_named(&Request::ProcKill { id: u32::MAX }).unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(payload, Request::ProcKill { id: u32::MAX });
        }
    }

    mod proc_stdin {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::ProcStdin {
                id: u32::MAX,
                data: vec![0, 1, 2, 3, u8::MAX],
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "proc_stdin",
                    "id": u32::MAX,
                    "data": [0, 1, 2, 3, u8::MAX],
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "proc_stdin",
                "id": u32::MAX,
                "data": [0, 1, 2, 3, u8::MAX],
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::ProcStdin {
                    id: u32::MAX,
                    data: vec![0, 1, 2, 3, u8::MAX],
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::ProcStdin {
                id: u32::MAX,
                data: vec![0, 1, 2, 3, u8::MAX],
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
            let buf = rmp_serde::encode::to_vec_named(&Request::ProcStdin {
                id: u32::MAX,
                data: vec![0, 1, 2, 3, u8::MAX],
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::ProcStdin {
                    id: u32::MAX,
                    data: vec![0, 1, 2, 3, u8::MAX],
                }
            );
        }
    }

    mod proc_resize_pty {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::ProcResizePty {
                id: u32::MAX,
                size: PtySize {
                    rows: u16::MAX,
                    cols: u16::MAX,
                    pixel_width: u16::MAX,
                    pixel_height: u16::MAX,
                },
            };

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "proc_resize_pty",
                    "id": u32::MAX,
                    "size": {
                        "rows": u16::MAX,
                        "cols": u16::MAX,
                        "pixel_width": u16::MAX,
                        "pixel_height": u16::MAX,
                    },
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "proc_resize_pty",
                "id": u32::MAX,
                "size": {
                    "rows": u16::MAX,
                    "cols": u16::MAX,
                    "pixel_width": u16::MAX,
                    "pixel_height": u16::MAX,
                },
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(
                payload,
                Request::ProcResizePty {
                    id: u32::MAX,
                    size: PtySize {
                        rows: u16::MAX,
                        cols: u16::MAX,
                        pixel_width: u16::MAX,
                        pixel_height: u16::MAX,
                    },
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::ProcResizePty {
                id: u32::MAX,
                size: PtySize {
                    rows: u16::MAX,
                    cols: u16::MAX,
                    pixel_width: u16::MAX,
                    pixel_height: u16::MAX,
                },
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
            let buf = rmp_serde::encode::to_vec_named(&Request::ProcResizePty {
                id: u32::MAX,
                size: PtySize {
                    rows: u16::MAX,
                    cols: u16::MAX,
                    pixel_width: u16::MAX,
                    pixel_height: u16::MAX,
                },
            })
            .unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                payload,
                Request::ProcResizePty {
                    id: u32::MAX,
                    size: PtySize {
                        rows: u16::MAX,
                        cols: u16::MAX,
                        pixel_width: u16::MAX,
                        pixel_height: u16::MAX,
                    },
                }
            );
        }
    }

    mod system_info {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::SystemInfo {};

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "system_info",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "system_info",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(payload, Request::SystemInfo {});
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::SystemInfo {};

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
            let buf = rmp_serde::encode::to_vec_named(&Request::SystemInfo {}).unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(payload, Request::SystemInfo {});
        }
    }

    mod version {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let payload = Request::Version {};

            let value = serde_json::to_value(payload).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "type": "version",
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "type": "version",
            });

            let payload: Request = serde_json::from_value(value).unwrap();
            assert_eq!(payload, Request::Version {});
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let payload = Request::Version {};

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
            let buf = rmp_serde::encode::to_vec_named(&Request::Version {}).unwrap();

            let payload: Request = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(payload, Request::Version {});
        }
    }
}
