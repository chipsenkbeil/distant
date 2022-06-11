use bitflags::bitflags;
use derive_more::{Deref, DerefMut, Display, Error, IntoIterator, IsVariant};
use notify::{
    event::Event as NotifyEvent, ErrorKind as NotifyErrorKind, EventKind as NotifyEventKind,
};
use portable_pty::PtySize as PortablePtySize;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet, io, iter::FromIterator, num::ParseIntError, ops::BitOr, path::PathBuf,
    str::FromStr,
};
use strum::{AsRefStr, EnumString, EnumVariantNames, VariantNames};

/// Type alias for a vec of bytes
///
/// NOTE: This only exists to support properly parsing a Vec<u8> from an entire string
///       with structopt rather than trying to parse a string as a singular u8
pub type ByteVec = Vec<u8>;

#[cfg(feature = "structopt")]
fn parse_byte_vec(src: &str) -> ByteVec {
    src.as_bytes().to_vec()
}

/// Represents the payload of a request to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[cfg_attr(feature = "structopt", derive(structopt::StructOpt))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
#[strum(serialize_all = "snake_case")]
pub enum DistantRequestData {
    /// Reads a file from the specified path on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["cat"]))]
    FileRead {
        /// The path to the file on the remote machine
        path: PathBuf,
    },

    /// Reads a file from the specified path on the remote machine
    /// and treats the contents as text
    FileReadText {
        /// The path to the file on the remote machine
        path: PathBuf,
    },

    /// Writes a file, creating it if it does not exist, and overwriting any existing content
    /// on the remote machine
    FileWrite {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content
        #[cfg_attr(feature = "structopt", structopt(parse(from_str = parse_byte_vec)))]
        data: ByteVec,
    },

    /// Writes a file using text instead of bytes, creating it if it does not exist,
    /// and overwriting any existing content on the remote machine
    FileWriteText {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content
        text: String,
    },

    /// Appends to a file, creating it if it does not exist, on the remote machine
    FileAppend {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content
        #[cfg_attr(feature = "structopt", structopt(parse(from_str = parse_byte_vec)))]
        data: ByteVec,
    },

    /// Appends text to a file, creating it if it does not exist, on the remote machine
    FileAppendText {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content
        text: String,
    },

    /// Reads a directory from the specified path on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["ls"]))]
    DirRead {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Maximum depth to traverse with 0 indicating there is no maximum
        /// depth and 1 indicating the most immediate children within the
        /// directory
        #[serde(default = "one")]
        #[cfg_attr(feature = "structopt", structopt(short, long, default_value = "1"))]
        depth: usize,

        /// Whether or not to return absolute or relative paths
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(short, long))]
        absolute: bool,

        /// Whether or not to canonicalize the resulting paths, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        ///
        /// Note that the flag absolute must be true to have absolute paths
        /// returned, even if canonicalize is flagged as true
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(short, long))]
        canonicalize: bool,

        /// Whether or not to include the root directory in the retrieved
        /// entries
        ///
        /// If included, the root directory will also be a canonicalized,
        /// absolute path and will not follow any of the other flags
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(long))]
        include_root: bool,
    },

    /// Creates a directory on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["mkdir"]))]
    DirCreate {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Whether or not to create all parent directories
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(short, long))]
        all: bool,
    },

    /// Removes a file or directory on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["rm"]))]
    Remove {
        /// The path to the file or directory on the remote machine
        path: PathBuf,

        /// Whether or not to remove all contents within directory if is a directory.
        /// Does nothing different for files
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(short, long))]
        force: bool,
    },

    /// Copies a file or directory on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["cp"]))]
    Copy {
        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for copy of file or directory
        dst: PathBuf,
    },

    /// Moves/renames a file or directory on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["mv"]))]
    Rename {
        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for the file or directory
        dst: PathBuf,
    },

    /// Watches a path for changes
    Watch {
        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,

        /// If true, will recursively watch for changes within directories, othewise
        /// will only watch for changes immediately within directories
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(short, long))]
        recursive: bool,

        /// Filter to only report back specified changes
        #[serde(default)]
        #[cfg_attr(
            feature = "structopt",
            structopt(short, long, possible_values = &ChangeKind::VARIANTS)
        )]
        only: Vec<ChangeKind>,

        /// Filter to report back changes except these specified changes
        #[serde(default)]
        #[cfg_attr(
            feature = "structopt", 
            structopt(short, long, possible_values = &ChangeKind::VARIANTS)
        )]
        except: Vec<ChangeKind>,
    },

    /// Unwatches a path for changes, meaning no additional changes will be reported
    Unwatch {
        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,
    },

    /// Checks whether the given path exists
    Exists {
        /// The path to the file or directory on the remote machine
        path: PathBuf,
    },

    /// Retrieves filesystem metadata for the specified path on the remote machine
    Metadata {
        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,

        /// Whether or not to include a canonicalized version of the path, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(short, long))]
        canonicalize: bool,

        /// Whether or not to follow symlinks to determine absolute file type (dir/file)
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(long))]
        resolve_file_type: bool,
    },

    /// Spawns a new process on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["spawn", "run"]))]
    ProcSpawn {
        /// The full command to run including arguments
        cmd: String,

        /// Whether or not the process should be persistent, meaning that the process will not be
        /// killed when the associated client disconnects
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(long))]
        persist: bool,

        /// If provided, will spawn process in a pty, otherwise spawns directly
        #[serde(default)]
        #[cfg_attr(feature = "structopt", structopt(long))]
        pty: Option<PtySize>,
    },

    /// Kills a process running on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["kill"]))]
    ProcKill {
        /// Id of the actively-running process
        id: usize,
    },

    /// Sends additional data to stdin of running process
    ProcStdin {
        /// Id of the actively-running process to send stdin data
        id: usize,

        /// Data to send to a process's stdin pipe
        data: Vec<u8>,
    },

    /// Resize pty of remote process
    ProcResizePty {
        /// Id of the actively-running process whose pty to resize
        id: usize,

        /// The new pty dimensions
        size: PtySize,
    },

    /// Retrieve a list of all processes being managed by the remote server
    ProcList {},

    /// Retrieve information about the server and the system it is on
    SystemInfo {},
}

/// Represents the payload of a successful response
#[derive(Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
#[strum(serialize_all = "snake_case")]
pub enum DistantResponseData {
    /// General okay with no extra data, returned in cases like
    /// creating or removing a directory, copying a file, or renaming
    /// a file
    Ok,

    /// General-purpose failure that occurred from some request
    Error(Error),

    /// Response containing some arbitrary, binary data
    Blob {
        /// Binary data associated with the response
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

    /// Response to starting a new process
    ProcSpawned {
        /// Arbitrary id associated with running process
        id: usize,
    },

    /// Actively-transmitted stdout as part of running process
    ProcStdout {
        /// Arbitrary id associated with running process
        id: usize,

        /// Data read from a process' stdout pipe
        data: Vec<u8>,
    },

    /// Actively-transmitted stderr as part of running process
    ProcStderr {
        /// Arbitrary id associated with running process
        id: usize,

        /// Data read from a process' stderr pipe
        data: Vec<u8>,
    },

    /// Response to a process finishing
    ProcDone {
        /// Arbitrary id associated with running process
        id: usize,

        /// Whether or not termination was successful
        success: bool,

        /// Exit code associated with termination, will be missing if terminated by signal
        code: Option<i32>,
    },

    /// Response to retrieving a list of managed processes
    ProcEntries {
        /// List of managed processes
        entries: Vec<RunningProcess>,
    },

    /// Response to retrieving information about the server and the system it is on
    SystemInfo(SystemInfo),
}

/// Represents the size associated with a remote PTY
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PtySize {
    /// Number of lines of text
    pub rows: u16,

    /// Number of columns of text
    pub cols: u16,

    /// Width of a cell in pixels. Note that some systems never fill this value and ignore it.
    #[serde(default)]
    pub pixel_width: u16,

    /// Height of a cell in pixels. Note that some systems never fill this value and ignore it.
    #[serde(default)]
    pub pixel_height: u16,
}

impl PtySize {
    /// Creates new size using just rows and columns
    pub fn from_rows_and_cols(rows: u16, cols: u16) -> Self {
        Self {
            rows,
            cols,
            ..Default::default()
        }
    }
}

impl From<PortablePtySize> for PtySize {
    fn from(size: PortablePtySize) -> Self {
        Self {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        }
    }
}

impl From<PtySize> for PortablePtySize {
    fn from(size: PtySize) -> Self {
        Self {
            rows: size.rows,
            cols: size.cols,
            pixel_width: size.pixel_width,
            pixel_height: size.pixel_height,
        }
    }
}

impl Default for PtySize {
    fn default() -> Self {
        PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Display, Error)]
pub enum PtySizeParseError {
    MissingRows,
    MissingColumns,
    InvalidRows(ParseIntError),
    InvalidColumns(ParseIntError),
    InvalidPixelWidth(ParseIntError),
    InvalidPixelHeight(ParseIntError),
}

impl FromStr for PtySize {
    type Err = PtySizeParseError;

    /// Attempts to parse a str into PtySize using one of the following formats:
    ///
    /// * rows,cols (defaults to 0 for pixel_width & pixel_height)
    /// * rows,cols,pixel_width,pixel_height
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.split(',');

        Ok(Self {
            rows: tokens
                .next()
                .ok_or(PtySizeParseError::MissingRows)?
                .trim()
                .parse()
                .map_err(PtySizeParseError::InvalidRows)?,
            cols: tokens
                .next()
                .ok_or(PtySizeParseError::MissingColumns)?
                .trim()
                .parse()
                .map_err(PtySizeParseError::InvalidColumns)?,
            pixel_width: tokens
                .next()
                .map(|s| s.trim().parse())
                .transpose()
                .map_err(PtySizeParseError::InvalidPixelWidth)?
                .unwrap_or(0),
            pixel_height: tokens
                .next()
                .map(|s| s.trim().parse())
                .transpose()
                .map_err(PtySizeParseError::InvalidPixelHeight)?
                .unwrap_or(0),
        })
    }
}

/// Represents metadata about some path on a remote machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Represents unix-specific metadata about some path on a remote machine
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

        flags.bits
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

        flags.bits
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

pub(crate) fn deserialize_u128_option<'de, D>(deserializer: D) -> Result<Option<u128>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<String>::deserialize(deserializer)? {
        Some(s) => match s.parse::<u128>() {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(serde::de::Error::custom(format!(
                "Cannot convert to u128 with error: {:?}",
                error
            ))),
        },
        None => Ok(None),
    }
}

pub(crate) fn serialize_u128_option<S: serde::Serializer>(
    val: &Option<u128>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match val {
        Some(v) => format!("{}", *v).serialize(s),
        None => s.serialize_unit(),
    }
}

/// Represents information about a system
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemInfo {
    /// Family of the operating system as described in
    /// https://doc.rust-lang.org/std/env/consts/constant.FAMILY.html
    pub family: String,

    /// Name of the specific operating system as described in
    /// https://doc.rust-lang.org/std/env/consts/constant.OS.html
    pub os: String,

    /// Architecture of the CPI as described in
    /// https://doc.rust-lang.org/std/env/consts/constant.ARCH.html
    pub arch: String,

    /// Current working directory of the running server process
    pub current_dir: PathBuf,

    /// Primary separator for path components for the current platform
    /// as defined in https://doc.rust-lang.org/std/path/constant.MAIN_SEPARATOR.html
    pub main_separator: char,
}

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
#[derive(Copy, Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum FileType {
    Dir,
    File,
    Symlink,
}

/// Represents information about a running process
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RunningProcess {
    /// Name of the command being run
    pub cmd: String,

    /// Arguments for the command
    pub args: Vec<String>,

    /// Whether or not the process was run in persist mode
    pub persist: bool,

    /// Pty associated with running process if it has one
    pub pty: Option<PtySize>,

    /// Arbitrary id associated with running process
    ///
    /// Not the same as the process' pid!
    pub id: usize,
}

impl From<io::Error> for DistantResponseData {
    fn from(x: io::Error) -> Self {
        Self::Error(Error::from(x))
    }
}

impl From<walkdir::Error> for DistantResponseData {
    fn from(x: walkdir::Error) -> Self {
        Self::Error(Error::from(x))
    }
}

impl From<notify::Error> for DistantResponseData {
    fn from(x: notify::Error) -> Self {
        Self::Error(Error::from(x))
    }
}

impl From<tokio::task::JoinError> for DistantResponseData {
    fn from(x: tokio::task::JoinError) -> Self {
        Self::Error(Error::from(x))
    }
}

/// Change to one or more paths on the filesystem
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Change {
    /// Label describing the kind of change
    pub kind: ChangeKind,

    /// Paths that were changed
    pub paths: Vec<PathBuf>,
}

impl From<NotifyEvent> for Change {
    fn from(x: NotifyEvent) -> Self {
        Self {
            kind: x.kind.into(),
            paths: x.paths,
        }
    }
}

#[derive(
    Copy,
    Clone,
    Debug,
    strum::Display,
    EnumString,
    EnumVariantNames,
    Hash,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[strum(serialize_all = "snake_case")]
pub enum ChangeKind {
    /// Something about a file or directory was accessed, but
    /// no specific details were known
    Access,

    /// A file was closed for executing
    AccessCloseExecute,

    /// A file was closed for reading
    AccessCloseRead,

    /// A file was closed for writing
    AccessCloseWrite,

    /// A file was opened for executing
    AccessOpenExecute,

    /// A file was opened for reading
    AccessOpenRead,

    /// A file was opened for writing
    AccessOpenWrite,

    /// A file or directory was read
    AccessRead,

    /// The access time of a file or directory was changed
    AccessTime,

    /// A file, directory, or something else was created
    Create,

    /// The content of a file or directory changed
    Content,

    /// The data of a file or directory was modified, but
    /// no specific details were known
    Data,

    /// The metadata of a file or directory was modified, but
    /// no specific details were known
    Metadata,

    /// Something about a file or directory was modified, but
    /// no specific details were known
    Modify,

    /// A file, directory, or something else was removed
    Remove,

    /// A file or directory was renamed, but no specific details were known
    Rename,

    /// A file or directory was renamed, and the provided paths
    /// are the source and target in that order (from, to)
    RenameBoth,

    /// A file or directory was renamed, and the provided path
    /// is the origin of the rename (before being renamed)
    RenameFrom,

    /// A file or directory was renamed, and the provided path
    /// is the result of the rename
    RenameTo,

    /// A file's size changed
    Size,

    /// The ownership of a file or directory was changed
    Ownership,

    /// The permissions of a file or directory was changed
    Permissions,

    /// The write or modify time of a file or directory was changed
    WriteTime,

    // Catchall in case we have no insight as to the type of change
    Unknown,
}

impl ChangeKind {
    /// Returns true if the change is a kind of access
    pub fn is_access_kind(&self) -> bool {
        self.is_open_access_kind()
            || self.is_close_access_kind()
            || matches!(self, Self::Access | Self::AccessRead)
    }

    /// Returns true if the change is a kind of open access
    pub fn is_open_access_kind(&self) -> bool {
        matches!(
            self,
            Self::AccessOpenExecute | Self::AccessOpenRead | Self::AccessOpenWrite
        )
    }

    /// Returns true if the change is a kind of close access
    pub fn is_close_access_kind(&self) -> bool {
        matches!(
            self,
            Self::AccessCloseExecute | Self::AccessCloseRead | Self::AccessCloseWrite
        )
    }

    /// Returns true if the change is a kind of creation
    pub fn is_create_kind(&self) -> bool {
        matches!(self, Self::Create)
    }

    /// Returns true if the change is a kind of modification
    pub fn is_modify_kind(&self) -> bool {
        self.is_data_modify_kind() || self.is_metadata_modify_kind() || matches!(self, Self::Modify)
    }

    /// Returns true if the change is a kind of data modification
    pub fn is_data_modify_kind(&self) -> bool {
        matches!(self, Self::Content | Self::Data | Self::Size)
    }

    /// Returns true if the change is a kind of metadata modification
    pub fn is_metadata_modify_kind(&self) -> bool {
        matches!(
            self,
            Self::AccessTime
                | Self::Metadata
                | Self::Ownership
                | Self::Permissions
                | Self::WriteTime
        )
    }

    /// Returns true if the change is a kind of rename
    pub fn is_rename_kind(&self) -> bool {
        matches!(
            self,
            Self::Rename | Self::RenameBoth | Self::RenameFrom | Self::RenameTo
        )
    }

    /// Returns true if the change is a kind of removal
    pub fn is_remove_kind(&self) -> bool {
        matches!(self, Self::Remove)
    }

    /// Returns true if the change kind is unknown
    pub fn is_unknown_kind(&self) -> bool {
        matches!(self, Self::Unknown)
    }
}

impl BitOr for ChangeKind {
    type Output = ChangeKindSet;

    fn bitor(self, rhs: Self) -> Self::Output {
        let mut set = ChangeKindSet::empty();
        set.insert(self);
        set.insert(rhs);
        set
    }
}

impl From<NotifyEventKind> for ChangeKind {
    fn from(x: NotifyEventKind) -> Self {
        use notify::event::{
            AccessKind, AccessMode, DataChange, MetadataKind, ModifyKind, RenameMode,
        };
        match x {
            // File/directory access events
            NotifyEventKind::Access(AccessKind::Read) => Self::AccessRead,
            NotifyEventKind::Access(AccessKind::Open(AccessMode::Execute)) => {
                Self::AccessOpenExecute
            }
            NotifyEventKind::Access(AccessKind::Open(AccessMode::Read)) => Self::AccessOpenRead,
            NotifyEventKind::Access(AccessKind::Open(AccessMode::Write)) => Self::AccessOpenWrite,
            NotifyEventKind::Access(AccessKind::Close(AccessMode::Execute)) => {
                Self::AccessCloseExecute
            }
            NotifyEventKind::Access(AccessKind::Close(AccessMode::Read)) => Self::AccessCloseRead,
            NotifyEventKind::Access(AccessKind::Close(AccessMode::Write)) => Self::AccessCloseWrite,
            NotifyEventKind::Access(_) => Self::Access,

            // File/directory creation events
            NotifyEventKind::Create(_) => Self::Create,

            // Rename-oriented events
            NotifyEventKind::Modify(ModifyKind::Name(RenameMode::Both)) => Self::RenameBoth,
            NotifyEventKind::Modify(ModifyKind::Name(RenameMode::From)) => Self::RenameFrom,
            NotifyEventKind::Modify(ModifyKind::Name(RenameMode::To)) => Self::RenameTo,
            NotifyEventKind::Modify(ModifyKind::Name(_)) => Self::Rename,

            // Data-modification events
            NotifyEventKind::Modify(ModifyKind::Data(DataChange::Content)) => Self::Content,
            NotifyEventKind::Modify(ModifyKind::Data(DataChange::Size)) => Self::Size,
            NotifyEventKind::Modify(ModifyKind::Data(_)) => Self::Data,

            // Metadata-modification events
            NotifyEventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)) => {
                Self::AccessTime
            }
            NotifyEventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)) => {
                Self::WriteTime
            }
            NotifyEventKind::Modify(ModifyKind::Metadata(MetadataKind::Permissions)) => {
                Self::Permissions
            }
            NotifyEventKind::Modify(ModifyKind::Metadata(MetadataKind::Ownership)) => {
                Self::Ownership
            }
            NotifyEventKind::Modify(ModifyKind::Metadata(_)) => Self::Metadata,

            // General modification events
            NotifyEventKind::Modify(_) => Self::Modify,

            // File/directory removal events
            NotifyEventKind::Remove(_) => Self::Remove,

            // Catch-all for other events
            NotifyEventKind::Any | NotifyEventKind::Other => Self::Unknown,
        }
    }
}

/// Represents a distinct set of different change kinds
#[derive(
    Clone, Debug, Deref, DerefMut, Display, IntoIterator, PartialEq, Eq, Serialize, Deserialize,
)]
#[display(
    fmt = "{}",
    "_0.iter().map(ToString::to_string).collect::<Vec<String>>().join(\",\")"
)]
pub struct ChangeKindSet(HashSet<ChangeKind>);

impl ChangeKindSet {
    /// Produces an empty set of [`ChangeKind`]
    pub fn empty() -> Self {
        Self(HashSet::new())
    }

    /// Produces a set of all [`ChangeKind`]
    pub fn all() -> Self {
        vec![
            ChangeKind::Access,
            ChangeKind::AccessCloseExecute,
            ChangeKind::AccessCloseRead,
            ChangeKind::AccessCloseWrite,
            ChangeKind::AccessOpenExecute,
            ChangeKind::AccessOpenRead,
            ChangeKind::AccessOpenWrite,
            ChangeKind::AccessRead,
            ChangeKind::AccessTime,
            ChangeKind::Create,
            ChangeKind::Content,
            ChangeKind::Data,
            ChangeKind::Metadata,
            ChangeKind::Modify,
            ChangeKind::Remove,
            ChangeKind::Rename,
            ChangeKind::RenameBoth,
            ChangeKind::RenameFrom,
            ChangeKind::RenameTo,
            ChangeKind::Size,
            ChangeKind::Ownership,
            ChangeKind::Permissions,
            ChangeKind::WriteTime,
            ChangeKind::Unknown,
        ]
        .into_iter()
        .collect()
    }

    /// Produces a changeset containing all of the access kinds
    pub fn access_set() -> Self {
        Self::access_open_set()
            | Self::access_close_set()
            | ChangeKind::AccessRead
            | ChangeKind::Access
    }

    /// Produces a changeset containing all of the open access kinds
    pub fn access_open_set() -> Self {
        ChangeKind::AccessOpenExecute | ChangeKind::AccessOpenRead | ChangeKind::AccessOpenWrite
    }

    /// Produces a changeset containing all of the close access kinds
    pub fn access_close_set() -> Self {
        ChangeKind::AccessCloseExecute | ChangeKind::AccessCloseRead | ChangeKind::AccessCloseWrite
    }

    // Produces a changeset containing all of the modification kinds
    pub fn modify_set() -> Self {
        Self::modify_data_set() | Self::modify_metadata_set() | ChangeKind::Modify
    }

    /// Produces a changeset containing all of the data modification kinds
    pub fn modify_data_set() -> Self {
        ChangeKind::Content | ChangeKind::Data | ChangeKind::Size
    }

    /// Produces a changeset containing all of the metadata modification kinds
    pub fn modify_metadata_set() -> Self {
        ChangeKind::AccessTime
            | ChangeKind::Metadata
            | ChangeKind::Ownership
            | ChangeKind::Permissions
            | ChangeKind::WriteTime
    }

    /// Produces a changeset containing all of the rename kinds
    pub fn rename_set() -> Self {
        ChangeKind::Rename | ChangeKind::RenameBoth | ChangeKind::RenameFrom | ChangeKind::RenameTo
    }

    /// Consumes set and returns a vec of the kinds of changes
    pub fn into_vec(self) -> Vec<ChangeKind> {
        self.0.into_iter().collect()
    }
}

impl BitOr<ChangeKindSet> for ChangeKindSet {
    type Output = Self;

    fn bitor(mut self, rhs: ChangeKindSet) -> Self::Output {
        self.extend(rhs.0);
        self
    }
}

impl BitOr<ChangeKind> for ChangeKindSet {
    type Output = Self;

    fn bitor(mut self, rhs: ChangeKind) -> Self::Output {
        self.0.insert(rhs);
        self
    }
}

impl BitOr<ChangeKindSet> for ChangeKind {
    type Output = ChangeKindSet;

    fn bitor(self, rhs: ChangeKindSet) -> Self::Output {
        rhs | self
    }
}

impl FromStr for ChangeKindSet {
    type Err = strum::ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut change_set = HashSet::new();

        for word in s.split(',') {
            change_set.insert(ChangeKind::from_str(word.trim())?);
        }

        Ok(ChangeKindSet(change_set))
    }
}

impl FromIterator<ChangeKind> for ChangeKindSet {
    fn from_iter<I: IntoIterator<Item = ChangeKind>>(iter: I) -> Self {
        let mut change_set = HashSet::new();

        for i in iter {
            change_set.insert(i);
        }

        ChangeKindSet(change_set)
    }
}

impl From<ChangeKind> for ChangeKindSet {
    fn from(change_kind: ChangeKind) -> Self {
        let mut set = Self::empty();
        set.insert(change_kind);
        set
    }
}

impl From<Vec<ChangeKind>> for ChangeKindSet {
    fn from(changes: Vec<ChangeKind>) -> Self {
        changes.into_iter().collect()
    }
}

impl Default for ChangeKindSet {
    fn default() -> Self {
        Self::empty()
    }
}

/// General purpose error type that can be sent across the wire
#[derive(Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[display(fmt = "{}: {}", kind, description)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Error {
    /// Label describing the kind of error
    pub kind: ErrorKind,

    /// Description of the error itself
    pub description: String,
}

impl std::error::Error for Error {}

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
            description: format!("{}", x),
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
            io::ErrorKind::UnexpectedEof => Self::UnexpectedEof,

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
            ErrorKind::UnexpectedEof => Self::UnexpectedEof,
            _ => Self::Other,
        }
    }
}

/// Used to provide a default serde value of 1
const fn one() -> usize {
    1
}
