use std::fs::FileType as StdFileType;
use std::path::PathBuf;

use derive_more::IsVariant;
use serde::{Deserialize, Serialize};
use strum::AsRefStr;

/// Represents information about a single entry within a directory
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct DirEntry {
    /// Represents the full path to the entry
    pub path: PathBuf,

    /// Represents the type of the entry as a file/dir/symlink
    pub file_type: FileType,

    /// Depth at which this entry was created relative to the root (0 being immediately within
    /// root)
    pub depth: usize,
}

/// Represents the type associated with a dir entry
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum FileType {
    Dir,
    File,
    Symlink,
}

impl From<StdFileType> for FileType {
    fn from(ft: StdFileType) -> Self {
        if ft.is_dir() {
            Self::Dir
        } else if ft.is_symlink() {
            Self::Symlink
        } else {
            Self::File
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod dir_entry {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let entry = DirEntry {
                path: PathBuf::from("dir").join("file"),
                file_type: FileType::File,
                depth: 1,
            };

            let path = entry.path.to_str().unwrap().to_string();
            let value = serde_json::to_value(entry).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "path": path,
                    "file_type": "file",
                    "depth": 1,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "path": "test-file",
                "file_type": "file",
                "depth": 0,
            });

            let entry: DirEntry = serde_json::from_value(value).unwrap();
            assert_eq!(
                entry,
                DirEntry {
                    path: PathBuf::from("test-file"),
                    file_type: FileType::File,
                    depth: 0,
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let entry = DirEntry {
                path: PathBuf::from("dir").join("file"),
                file_type: FileType::File,
                depth: 1,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&entry).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&DirEntry {
                path: PathBuf::from("test-file"),
                file_type: FileType::File,
                depth: 0,
            })
            .unwrap();

            let entry: DirEntry = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                entry,
                DirEntry {
                    path: PathBuf::from("test-file"),
                    file_type: FileType::File,
                    depth: 0,
                }
            );
        }
    }

    mod file_type {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let ty = FileType::File;

            let value = serde_json::to_value(ty).unwrap();
            assert_eq!(value, serde_json::json!("file"));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!("file");

            let ty: FileType = serde_json::from_value(value).unwrap();
            assert_eq!(ty, FileType::File);
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let ty = FileType::File;

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&ty).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&FileType::File).unwrap();

            let ty: FileType = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(ty, FileType::File);
        }
    }
}
