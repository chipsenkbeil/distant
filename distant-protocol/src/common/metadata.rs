use std::path::PathBuf;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::common::FileType;
use crate::utils::{deserialize_u128_option, serialize_u128_option};

/// Represents metadata about some path on a remote machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// Canonicalized path to the file or directory, resolving symlinks, only included if flagged
    /// during the request.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub canonicalized_path: Option<PathBuf>,

    /// Represents the type of the entry as a file/dir/symlink.
    pub file_type: FileType,

    /// Size of the file/directory/symlink in bytes.
    pub len: u64,

    /// Whether or not the file/directory/symlink is marked as unwriteable.
    pub readonly: bool,

    /// Represents the last time (in milliseconds) when the file/directory/symlink was accessed;
    /// can be optional as certain systems don't support this.
    ///
    /// Note that this is represented as a string and not a number when serialized!
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_u128_option")]
    #[serde(deserialize_with = "deserialize_u128_option")]
    #[serde(default)]
    pub accessed: Option<u128>,

    /// Represents when (in milliseconds) the file/directory/symlink was created;
    /// can be optional as certain systems don't support this.
    ///
    /// Note that this is represented as a string and not a number when serialized!
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_u128_option")]
    #[serde(deserialize_with = "deserialize_u128_option")]
    #[serde(default)]
    pub created: Option<u128>,

    /// Represents the last time (in milliseconds) when the file/directory/symlink was modified;
    /// can be optional as certain systems don't support this.
    ///
    /// Note that this is represented as a string and not a number when serialized!
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_u128_option")]
    #[serde(deserialize_with = "deserialize_u128_option")]
    #[serde(default)]
    pub modified: Option<u128>,

    /// Represents metadata that is specific to a unix remote machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub unix: Option<UnixMetadata>,

    /// Represents metadata that is specific to a windows remote machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub windows: Option<WindowsMetadata>,
}

/// Represents unix-specific metadata about some path on a remote machine.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnixMetadata {
    /// Represents whether or not owner can read from the file.
    pub owner_read: bool,

    /// Represents whether or not owner can write to the file.
    pub owner_write: bool,

    /// Represents whether or not owner can execute the file.
    pub owner_exec: bool,

    /// Represents whether or not associated group can read from the file.
    pub group_read: bool,

    /// Represents whether or not associated group can write to the file.
    pub group_write: bool,

    /// Represents whether or not associated group can execute the file.
    pub group_exec: bool,

    /// Represents whether or not other can read from the file.
    pub other_read: bool,

    /// Represents whether or not other can write to the file.
    pub other_write: bool,

    /// Represents whether or not other can execute the file.
    pub other_exec: bool,
}

impl From<u32> for UnixMetadata {
    /// Create from a unix mode bitset
    fn from(mode: u32) -> Self {
        let flags = UnixFilePermissionFlags::from_bits_truncate(mode);
        Self {
            owner_read: flags.contains(UnixFilePermissionFlags::OWNER_READ),
            owner_write: flags.contains(UnixFilePermissionFlags::OWNER_WRITE),
            owner_exec: flags.contains(UnixFilePermissionFlags::OWNER_EXEC),
            group_read: flags.contains(UnixFilePermissionFlags::GROUP_READ),
            group_write: flags.contains(UnixFilePermissionFlags::GROUP_WRITE),
            group_exec: flags.contains(UnixFilePermissionFlags::GROUP_EXEC),
            other_read: flags.contains(UnixFilePermissionFlags::OTHER_READ),
            other_write: flags.contains(UnixFilePermissionFlags::OTHER_WRITE),
            other_exec: flags.contains(UnixFilePermissionFlags::OTHER_EXEC),
        }
    }
}

impl From<UnixMetadata> for u32 {
    /// Convert to a unix mode bitset.
    fn from(metadata: UnixMetadata) -> Self {
        let mut flags = UnixFilePermissionFlags::empty();

        if metadata.owner_read {
            flags.insert(UnixFilePermissionFlags::OWNER_READ);
        }
        if metadata.owner_write {
            flags.insert(UnixFilePermissionFlags::OWNER_WRITE);
        }
        if metadata.owner_exec {
            flags.insert(UnixFilePermissionFlags::OWNER_EXEC);
        }

        if metadata.group_read {
            flags.insert(UnixFilePermissionFlags::GROUP_READ);
        }
        if metadata.group_write {
            flags.insert(UnixFilePermissionFlags::GROUP_WRITE);
        }
        if metadata.group_exec {
            flags.insert(UnixFilePermissionFlags::GROUP_EXEC);
        }

        if metadata.other_read {
            flags.insert(UnixFilePermissionFlags::OTHER_READ);
        }
        if metadata.other_write {
            flags.insert(UnixFilePermissionFlags::OTHER_WRITE);
        }
        if metadata.other_exec {
            flags.insert(UnixFilePermissionFlags::OTHER_EXEC);
        }

        flags.bits()
    }
}

impl UnixMetadata {
    pub fn is_readonly(self) -> bool {
        !(self.owner_write || self.group_write || self.other_write)
    }
}

bitflags! {
    struct UnixFilePermissionFlags: u32 {
        const OWNER_READ = 0o400;
        const OWNER_WRITE = 0o200;
        const OWNER_EXEC = 0o100;
        const GROUP_READ = 0o40;
        const GROUP_WRITE = 0o20;
        const GROUP_EXEC = 0o10;
        const OTHER_READ = 0o4;
        const OTHER_WRITE = 0o2;
        const OTHER_EXEC = 0o1;
    }
}

/// Represents windows-specific metadata about some path on a remote machine
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowsMetadata {
    /// Represents whether or not a file or directory is an archive
    pub archive: bool,

    /// Represents whether or not a file or directory is compressed
    pub compressed: bool,

    /// Represents whether or not the file or directory is encrypted
    pub encrypted: bool,

    /// Represents whether or not a file or directory is hidden
    pub hidden: bool,

    /// Represents whether or not a directory or user data stream is configured with integrity
    pub integrity_stream: bool,

    /// Represents whether or not a file does not have other attributes set
    pub normal: bool,

    /// Represents whether or not a file or directory is not to be indexed by content indexing
    /// service
    pub not_content_indexed: bool,

    /// Represents whether or not a user data stream is not to be read by the background data
    /// integrity scanner
    pub no_scrub_data: bool,

    /// Represents whether or not the data of a file is not available immediately
    pub offline: bool,

    /// Represents whether or not a file or directory is not fully present locally
    pub recall_on_data_access: bool,

    /// Represents whether or not a file or directory has no physical representation on the local
    /// system (is virtual)
    pub recall_on_open: bool,

    /// Represents whether or not a file or directory has an associated reparse point, or a file is
    /// a symbolic link
    pub reparse_point: bool,

    /// Represents whether or not a file is a sparse file
    pub sparse_file: bool,

    /// Represents whether or not a file or directory is used partially or exclusively by the
    /// operating system
    pub system: bool,

    /// Represents whether or not a file is being used for temporary storage
    pub temporary: bool,
}

impl From<u32> for WindowsMetadata {
    /// Create from a windows file attribute bitset
    fn from(file_attributes: u32) -> Self {
        let flags = WindowsFileAttributeFlags::from_bits_truncate(file_attributes);
        Self {
            archive: flags.contains(WindowsFileAttributeFlags::ARCHIVE),
            compressed: flags.contains(WindowsFileAttributeFlags::COMPRESSED),
            encrypted: flags.contains(WindowsFileAttributeFlags::ENCRYPTED),
            hidden: flags.contains(WindowsFileAttributeFlags::HIDDEN),
            integrity_stream: flags.contains(WindowsFileAttributeFlags::INTEGRITY_SYSTEM),
            normal: flags.contains(WindowsFileAttributeFlags::NORMAL),
            not_content_indexed: flags.contains(WindowsFileAttributeFlags::NOT_CONTENT_INDEXED),
            no_scrub_data: flags.contains(WindowsFileAttributeFlags::NO_SCRUB_DATA),
            offline: flags.contains(WindowsFileAttributeFlags::OFFLINE),
            recall_on_data_access: flags.contains(WindowsFileAttributeFlags::RECALL_ON_DATA_ACCESS),
            recall_on_open: flags.contains(WindowsFileAttributeFlags::RECALL_ON_OPEN),
            reparse_point: flags.contains(WindowsFileAttributeFlags::REPARSE_POINT),
            sparse_file: flags.contains(WindowsFileAttributeFlags::SPARSE_FILE),
            system: flags.contains(WindowsFileAttributeFlags::SYSTEM),
            temporary: flags.contains(WindowsFileAttributeFlags::TEMPORARY),
        }
    }
}

impl From<WindowsMetadata> for u32 {
    /// Convert to a windows file attribute bitset
    fn from(metadata: WindowsMetadata) -> Self {
        let mut flags = WindowsFileAttributeFlags::empty();

        if metadata.archive {
            flags.insert(WindowsFileAttributeFlags::ARCHIVE);
        }
        if metadata.compressed {
            flags.insert(WindowsFileAttributeFlags::COMPRESSED);
        }
        if metadata.encrypted {
            flags.insert(WindowsFileAttributeFlags::ENCRYPTED);
        }
        if metadata.hidden {
            flags.insert(WindowsFileAttributeFlags::HIDDEN);
        }
        if metadata.integrity_stream {
            flags.insert(WindowsFileAttributeFlags::INTEGRITY_SYSTEM);
        }
        if metadata.normal {
            flags.insert(WindowsFileAttributeFlags::NORMAL);
        }
        if metadata.not_content_indexed {
            flags.insert(WindowsFileAttributeFlags::NOT_CONTENT_INDEXED);
        }
        if metadata.no_scrub_data {
            flags.insert(WindowsFileAttributeFlags::NO_SCRUB_DATA);
        }
        if metadata.offline {
            flags.insert(WindowsFileAttributeFlags::OFFLINE);
        }
        if metadata.recall_on_data_access {
            flags.insert(WindowsFileAttributeFlags::RECALL_ON_DATA_ACCESS);
        }
        if metadata.recall_on_open {
            flags.insert(WindowsFileAttributeFlags::RECALL_ON_OPEN);
        }
        if metadata.reparse_point {
            flags.insert(WindowsFileAttributeFlags::REPARSE_POINT);
        }
        if metadata.sparse_file {
            flags.insert(WindowsFileAttributeFlags::SPARSE_FILE);
        }
        if metadata.system {
            flags.insert(WindowsFileAttributeFlags::SYSTEM);
        }
        if metadata.temporary {
            flags.insert(WindowsFileAttributeFlags::TEMPORARY);
        }

        flags.bits()
    }
}

bitflags! {
    struct WindowsFileAttributeFlags: u32 {
        const ARCHIVE = 0x20;
        const COMPRESSED = 0x800;
        const ENCRYPTED = 0x4000;
        const HIDDEN = 0x2;
        const INTEGRITY_SYSTEM = 0x8000;
        const NORMAL = 0x80;
        const NOT_CONTENT_INDEXED = 0x2000;
        const NO_SCRUB_DATA = 0x20000;
        const OFFLINE = 0x1000;
        const RECALL_ON_DATA_ACCESS = 0x400000;
        const RECALL_ON_OPEN = 0x40000;
        const REPARSE_POINT = 0x400;
        const SPARSE_FILE = 0x200;
        const SYSTEM = 0x4;
        const TEMPORARY = 0x100;
        const VIRTUAL = 0x10000;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod metadata {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_minimal_metadata_to_json() {
            let metadata = Metadata {
                canonicalized_path: None,
                file_type: FileType::Dir,
                len: 999,
                readonly: true,
                accessed: None,
                created: None,
                modified: None,
                unix: None,
                windows: None,
            };

            let value = serde_json::to_value(metadata).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "file_type": "dir",
                    "len": 999,
                    "readonly": true,
                })
            );
        }

        #[test]
        fn should_be_able_to_serialize_full_metadata_to_json() {
            let metadata = Metadata {
                canonicalized_path: Some(PathBuf::from("test-dir")),
                file_type: FileType::Dir,
                len: 999,
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
            };

            // NOTE: These values are too big to normally serialize, so we have to convert them to
            // a string type, which is why the value here also needs to be a string.
            let max_u128_str = u128::MAX.to_string();

            let value = serde_json::to_value(metadata).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "canonicalized_path": "test-dir",
                    "file_type": "dir",
                    "len": 999,
                    "readonly": true,
                    "accessed": max_u128_str,
                    "created": max_u128_str,
                    "modified": max_u128_str,
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
        fn should_be_able_to_deserialize_minimal_metadata_from_json() {
            let value = serde_json::json!({
                "file_type": "dir",
                "len": 999,
                "readonly": true,
            });

            let metadata: Metadata = serde_json::from_value(value).unwrap();
            assert_eq!(
                metadata,
                Metadata {
                    canonicalized_path: None,
                    file_type: FileType::Dir,
                    len: 999,
                    readonly: true,
                    accessed: None,
                    created: None,
                    modified: None,
                    unix: None,
                    windows: None,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_metadata_from_json() {
            // NOTE: These values are too big to normally serialize, so we have to convert them to
            // a string type, which is why the value here also needs to be a string.
            let max_u128_str = u128::MAX.to_string();

            let value = serde_json::json!({
                "canonicalized_path": "test-dir",
                "file_type": "dir",
                "len": 999,
                "readonly": true,
                "accessed": max_u128_str,
                "created": max_u128_str,
                "modified": max_u128_str,
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

            let metadata: Metadata = serde_json::from_value(value).unwrap();
            assert_eq!(
                metadata,
                Metadata {
                    canonicalized_path: Some(PathBuf::from("test-dir")),
                    file_type: FileType::Dir,
                    len: 999,
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
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_minimal_metadata_to_msgpack() {
            let metadata = Metadata {
                canonicalized_path: None,
                file_type: FileType::Dir,
                len: 999,
                readonly: true,
                accessed: None,
                created: None,
                modified: None,
                unix: None,
                windows: None,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&metadata).unwrap();
        }

        #[test]
        fn should_be_able_to_serialize_full_metadata_to_msgpack() {
            let metadata = Metadata {
                canonicalized_path: Some(PathBuf::from("test-dir")),
                file_type: FileType::Dir,
                len: 999,
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
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&metadata).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_minimal_metadata_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or preventing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Metadata {
                canonicalized_path: None,
                file_type: FileType::Dir,
                len: 999,
                readonly: true,
                accessed: None,
                created: None,
                modified: None,
                unix: None,
                windows: None,
            })
            .unwrap();

            let metadata: Metadata = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                metadata,
                Metadata {
                    canonicalized_path: None,
                    file_type: FileType::Dir,
                    len: 999,
                    readonly: true,
                    accessed: None,
                    created: None,
                    modified: None,
                    unix: None,
                    windows: None,
                }
            );
        }

        #[test]
        fn should_be_able_to_deserialize_full_metadata_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or preventing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Metadata {
                canonicalized_path: Some(PathBuf::from("test-dir")),
                file_type: FileType::Dir,
                len: 999,
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
            .unwrap();

            let metadata: Metadata = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                metadata,
                Metadata {
                    canonicalized_path: Some(PathBuf::from("test-dir")),
                    file_type: FileType::Dir,
                    len: 999,
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
                }
            );
        }
    }

    mod unix_metadata {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let metadata = UnixMetadata {
                owner_read: true,
                owner_write: false,
                owner_exec: false,
                group_read: true,
                group_write: false,
                group_exec: false,
                other_read: true,
                other_write: false,
                other_exec: false,
            };

            let value = serde_json::to_value(metadata).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "owner_read": true,
                    "owner_write": false,
                    "owner_exec": false,
                    "group_read": true,
                    "group_write": false,
                    "group_exec": false,
                    "other_read": true,
                    "other_write": false,
                    "other_exec": false,
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
                "owner_read": true,
                "owner_write": false,
                "owner_exec": false,
                "group_read": true,
                "group_write": false,
                "group_exec": false,
                "other_read": true,
                "other_write": false,
                "other_exec": false,
            });

            let metadata: UnixMetadata = serde_json::from_value(value).unwrap();
            assert_eq!(
                metadata,
                UnixMetadata {
                    owner_read: true,
                    owner_write: false,
                    owner_exec: false,
                    group_read: true,
                    group_write: false,
                    group_exec: false,
                    other_read: true,
                    other_write: false,
                    other_exec: false,
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let metadata = UnixMetadata {
                owner_read: true,
                owner_write: false,
                owner_exec: false,
                group_read: true,
                group_write: false,
                group_exec: false,
                other_read: true,
                other_write: false,
                other_exec: false,
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&metadata).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or preventing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&UnixMetadata {
                owner_read: true,
                owner_write: false,
                owner_exec: false,
                group_read: true,
                group_write: false,
                group_exec: false,
                other_read: true,
                other_write: false,
                other_exec: false,
            })
            .unwrap();

            let metadata: UnixMetadata = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                metadata,
                UnixMetadata {
                    owner_read: true,
                    owner_write: false,
                    owner_exec: false,
                    group_read: true,
                    group_write: false,
                    group_exec: false,
                    other_read: true,
                    other_write: false,
                    other_exec: false,
                }
            );
        }
    }

    mod windows_metadata {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let metadata = WindowsMetadata {
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
            };

            let value = serde_json::to_value(metadata).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
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
                })
            );
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({
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
            });

            let metadata: WindowsMetadata = serde_json::from_value(value).unwrap();
            assert_eq!(
                metadata,
                WindowsMetadata {
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
                }
            );
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let metadata = WindowsMetadata {
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
            };

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&metadata).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or preventing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&WindowsMetadata {
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
            })
            .unwrap();

            let metadata: WindowsMetadata = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                metadata,
                WindowsMetadata {
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
                }
            );
        }
    }
}
