use std::path::PathBuf;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::protocol::common::FileType;

/// Represents metadata about some path on a remote machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    /// Canonicalized path to the file or directory, resolving symlinks, only included if flagged
    /// during the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonicalized_path: Option<PathBuf>,

    /// Represents the type of the entry as a file/dir/symlink.
    pub file_type: FileType,

    /// Size of the file/directory/symlink in bytes.
    pub len: u64,

    /// Whether or not the file/directory/symlink is marked as unwriteable.
    pub readonly: bool,

    /// Represents the last time (in seconds) when the file/directory/symlink was accessed;
    /// can be optional as certain systems don't support this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessed: Option<u64>,

    /// Represents when (in seconds) the file/directory/symlink was created;
    /// can be optional as certain systems don't support this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,

    /// Represents the last time (in seconds) when the file/directory/symlink was modified;
    /// can be optional as certain systems don't support this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<u64>,

    /// Represents metadata that is specific to a unix remote machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unix: Option<UnixMetadata>,

    /// Represents metadata that is specific to a windows remote machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    //! Tests for UnixMetadata (permission bit round-trips, is_readonly) and WindowsMetadata
    //! (attribute flag round-trips for all 15 flags, individual and combined).

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
                accessed: Some(u64::MAX),
                created: Some(u64::MAX),
                modified: Some(u64::MAX),
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

            let value = serde_json::to_value(metadata).unwrap();
            assert_eq!(
                value,
                serde_json::json!({
                    "canonicalized_path": "test-dir",
                    "file_type": "dir",
                    "len": 999,
                    "readonly": true,
                    "accessed": u64::MAX,
                    "created": u64::MAX,
                    "modified": u64::MAX,
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
            let value = serde_json::json!({
                "canonicalized_path": "test-dir",
                "file_type": "dir",
                "len": 999,
                "readonly": true,
                "accessed": u64::MAX,
                "created": u64::MAX,
                "modified": u64::MAX,
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
                    accessed: Some(u64::MAX),
                    created: Some(u64::MAX),
                    modified: Some(u64::MAX),
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
                accessed: Some(u64::MAX),
                created: Some(u64::MAX),
                modified: Some(u64::MAX),
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
                accessed: Some(u64::MAX),
                created: Some(u64::MAX),
                modified: Some(u64::MAX),
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
                    accessed: Some(u64::MAX),
                    created: Some(u64::MAX),
                    modified: Some(u64::MAX),
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
        fn from_u32_to_unix_metadata_and_back_round_trip() {
            // 0o755 = owner rwx, group rx, other rx
            let mode: u32 = 0o755;
            let meta = UnixMetadata::from(mode);
            assert!(meta.owner_read);
            assert!(meta.owner_write);
            assert!(meta.owner_exec);
            assert!(meta.group_read);
            assert!(!meta.group_write);
            assert!(meta.group_exec);
            assert!(meta.other_read);
            assert!(!meta.other_write);
            assert!(meta.other_exec);

            // Round-trip back
            let back: u32 = meta.into();
            assert_eq!(back, mode);
        }

        #[test]
        fn from_u32_to_unix_metadata_0o644() {
            let mode: u32 = 0o644;
            let meta = UnixMetadata::from(mode);
            assert!(meta.owner_read);
            assert!(meta.owner_write);
            assert!(!meta.owner_exec);
            assert!(meta.group_read);
            assert!(!meta.group_write);
            assert!(!meta.group_exec);
            assert!(meta.other_read);
            assert!(!meta.other_write);
            assert!(!meta.other_exec);

            let back: u32 = meta.into();
            assert_eq!(back, mode);
        }

        #[test]
        fn from_u32_to_unix_metadata_0o000() {
            let mode: u32 = 0o000;
            let meta = UnixMetadata::from(mode);
            assert!(!meta.owner_read);
            assert!(!meta.owner_write);
            assert!(!meta.owner_exec);
            assert!(!meta.group_read);
            assert!(!meta.group_write);
            assert!(!meta.group_exec);
            assert!(!meta.other_read);
            assert!(!meta.other_write);
            assert!(!meta.other_exec);

            let back: u32 = meta.into();
            assert_eq!(back, mode);
        }

        #[test]
        fn from_u32_to_unix_metadata_0o777() {
            let mode: u32 = 0o777;
            let meta = UnixMetadata::from(mode);
            assert!(meta.owner_read);
            assert!(meta.owner_write);
            assert!(meta.owner_exec);
            assert!(meta.group_read);
            assert!(meta.group_write);
            assert!(meta.group_exec);
            assert!(meta.other_read);
            assert!(meta.other_write);
            assert!(meta.other_exec);

            let back: u32 = meta.into();
            assert_eq!(back, mode);
        }

        #[test]
        fn is_readonly_when_no_write_bits() {
            let meta = UnixMetadata::from(0o444u32);
            assert!(meta.is_readonly());
        }

        #[test]
        fn is_not_readonly_when_owner_write() {
            let meta = UnixMetadata::from(0o644u32);
            assert!(!meta.is_readonly());
        }

        #[test]
        fn is_not_readonly_when_group_write() {
            let meta = UnixMetadata::from(0o464u32);
            assert!(!meta.is_readonly());
        }

        #[test]
        fn is_not_readonly_when_other_write() {
            let meta = UnixMetadata::from(0o446u32);
            assert!(!meta.is_readonly());
        }

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
        fn from_u32_to_windows_metadata_and_back_round_trip() {
            // ARCHIVE(0x20) | HIDDEN(0x2) | SYSTEM(0x4) = 0x26
            let attrs: u32 = 0x26;
            let meta = WindowsMetadata::from(attrs);
            assert!(meta.archive);
            assert!(!meta.compressed);
            assert!(!meta.encrypted);
            assert!(meta.hidden);
            assert!(!meta.integrity_stream);
            assert!(!meta.normal);
            assert!(!meta.not_content_indexed);
            assert!(!meta.no_scrub_data);
            assert!(!meta.offline);
            assert!(!meta.recall_on_data_access);
            assert!(!meta.recall_on_open);
            assert!(!meta.reparse_point);
            assert!(!meta.sparse_file);
            assert!(meta.system);
            assert!(!meta.temporary);

            let back: u32 = meta.into();
            assert_eq!(back, attrs);
        }

        #[test]
        fn from_u32_to_windows_metadata_zero() {
            let meta = WindowsMetadata::from(0u32);
            assert!(!meta.archive);
            assert!(!meta.compressed);
            assert!(!meta.encrypted);
            assert!(!meta.hidden);
            assert!(!meta.integrity_stream);
            assert!(!meta.normal);
            assert!(!meta.not_content_indexed);
            assert!(!meta.no_scrub_data);
            assert!(!meta.offline);
            assert!(!meta.recall_on_data_access);
            assert!(!meta.recall_on_open);
            assert!(!meta.reparse_point);
            assert!(!meta.sparse_file);
            assert!(!meta.system);
            assert!(!meta.temporary);

            let back: u32 = meta.into();
            assert_eq!(back, 0);
        }

        #[test]
        fn from_u32_to_windows_metadata_all_flags() {
            // Set all known flags
            let all: u32 = 0x20
                | 0x800
                | 0x4000
                | 0x2
                | 0x8000
                | 0x80
                | 0x2000
                | 0x20000
                | 0x1000
                | 0x400000
                | 0x40000
                | 0x400
                | 0x200
                | 0x4
                | 0x100;
            let meta = WindowsMetadata::from(all);
            assert!(meta.archive);
            assert!(meta.compressed);
            assert!(meta.encrypted);
            assert!(meta.hidden);
            assert!(meta.integrity_stream);
            assert!(meta.normal);
            assert!(meta.not_content_indexed);
            assert!(meta.no_scrub_data);
            assert!(meta.offline);
            assert!(meta.recall_on_data_access);
            assert!(meta.recall_on_open);
            assert!(meta.reparse_point);
            assert!(meta.sparse_file);
            assert!(meta.system);
            assert!(meta.temporary);

            let back: u32 = meta.into();
            assert_eq!(back, all);
        }

        #[test]
        fn from_u32_to_windows_metadata_individual_flags() {
            // Test individual flags
            let test_cases: Vec<(u32, &str)> = vec![
                (0x20, "archive"),
                (0x800, "compressed"),
                (0x4000, "encrypted"),
                (0x2, "hidden"),
                (0x8000, "integrity_stream"),
                (0x80, "normal"),
                (0x2000, "not_content_indexed"),
                (0x20000, "no_scrub_data"),
                (0x1000, "offline"),
                (0x400000, "recall_on_data_access"),
                (0x40000, "recall_on_open"),
                (0x400, "reparse_point"),
                (0x200, "sparse_file"),
                (0x4, "system"),
                (0x100, "temporary"),
            ];

            for (flag, name) in test_cases {
                let meta = WindowsMetadata::from(flag);
                let back: u32 = meta.into();
                assert_eq!(back, flag, "Round-trip failed for {name}");
            }
        }

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
