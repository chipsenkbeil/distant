use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use super::{deserialize_u128_option, serialize_u128_option, FileType};

/// Represents metadata about some path on a remote machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Metadata {
    /// Canonicalized path to the file or directory, resolving symlinks, only included
    /// if flagged during the request
    pub canonicalized_path: Option<PathBuf>,

    /// Represents the type of the entry as a file/dir/symlink
    pub file_type: FileType,

    /// Size of the file/directory/symlink in bytes
    pub len: u64,

    /// Whether or not the file/directory/symlink is marked as unwriteable
    pub readonly: bool,

    /// Represents the last time (in milliseconds) when the file/directory/symlink was accessed;
    /// can be optional as certain systems don't support this
    #[serde(serialize_with = "serialize_u128_option")]
    #[serde(deserialize_with = "deserialize_u128_option")]
    pub accessed: Option<u128>,

    /// Represents when (in milliseconds) the file/directory/symlink was created;
    /// can be optional as certain systems don't support this
    #[serde(serialize_with = "serialize_u128_option")]
    #[serde(deserialize_with = "deserialize_u128_option")]
    pub created: Option<u128>,

    /// Represents the last time (in milliseconds) when the file/directory/symlink was modified;
    /// can be optional as certain systems don't support this
    #[serde(serialize_with = "serialize_u128_option")]
    #[serde(deserialize_with = "deserialize_u128_option")]
    pub modified: Option<u128>,

    /// Represents metadata that is specific to a unix remote machine
    pub unix: Option<UnixMetadata>,

    /// Represents metadata that is specific to a windows remote machine
    pub windows: Option<WindowsMetadata>,
}

impl Metadata {
    pub async fn read(
        path: impl AsRef<Path>,
        canonicalize: bool,
        resolve_file_type: bool,
    ) -> io::Result<Self> {
        let metadata = tokio::fs::symlink_metadata(path.as_ref()).await?;
        let canonicalized_path = if canonicalize {
            Some(tokio::fs::canonicalize(path.as_ref()).await?)
        } else {
            None
        };

        // If asking for resolved file type and current type is symlink, then we want to refresh
        // our metadata to get the filetype for the resolved link
        let file_type = if resolve_file_type && metadata.file_type().is_symlink() {
            tokio::fs::metadata(path).await?.file_type()
        } else {
            metadata.file_type()
        };

        Ok(Self {
            canonicalized_path,
            accessed: metadata
                .accessed()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_millis()),
            created: metadata
                .created()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_millis()),
            modified: metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_millis()),
            len: metadata.len(),
            readonly: metadata.permissions().readonly(),
            file_type: if file_type.is_dir() {
                FileType::Dir
            } else if file_type.is_file() {
                FileType::File
            } else {
                FileType::Symlink
            },

            #[cfg(unix)]
            unix: Some({
                use std::os::unix::prelude::*;
                let mode = metadata.mode();
                crate::data::UnixMetadata::from(mode)
            }),
            #[cfg(not(unix))]
            unix: None,

            #[cfg(windows)]
            windows: Some({
                use std::os::windows::prelude::*;
                let attributes = metadata.file_attributes();
                crate::data::WindowsMetadata::from(attributes)
            }),
            #[cfg(not(windows))]
            windows: None,
        })
    }
}

#[cfg(feature = "schemars")]
impl Metadata {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Metadata)
    }
}

/// Represents unix-specific metadata about some path on a remote machine
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct UnixMetadata {
    /// Represents whether or not owner can read from the file
    pub owner_read: bool,

    /// Represents whether or not owner can write to the file
    pub owner_write: bool,

    /// Represents whether or not owner can execute the file
    pub owner_exec: bool,

    /// Represents whether or not associated group can read from the file
    pub group_read: bool,

    /// Represents whether or not associated group can write to the file
    pub group_write: bool,

    /// Represents whether or not associated group can execute the file
    pub group_exec: bool,

    /// Represents whether or not other can read from the file
    pub other_read: bool,

    /// Represents whether or not other can write to the file
    pub other_write: bool,

    /// Represents whether or not other can execute the file
    pub other_exec: bool,
}

#[cfg(feature = "schemars")]
impl UnixMetadata {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(UnixMetadata)
    }
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
    /// Convert to a unix mode bitset
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
        !(self.owner_read || self.group_read || self.other_read)
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
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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

#[cfg(feature = "schemars")]
impl WindowsMetadata {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(WindowsMetadata)
    }
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
