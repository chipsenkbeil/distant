use derive_more::{Display, Error, IsVariant};
use serde::{Deserialize, Serialize};
use std::{io, num::ParseIntError, path::PathBuf, str::FromStr};
use strum::AsRefStr;

/// Type alias for a vec of bytes
///
/// NOTE: This only exists to support properly parsing a Vec<u8> from an entire string
///       with structopt rather than trying to parse a string as a singular u8
pub type ByteVec = Vec<u8>;

#[cfg(feature = "structopt")]
fn parse_byte_vec(src: &str) -> ByteVec {
    src.as_bytes().to_vec()
}

/// Represents the request to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Request {
    /// A name tied to the requester (tenant)
    pub tenant: String,

    /// A unique id associated with the request
    pub id: usize,

    /// The main payload containing a collection of data comprising one or more actions
    pub payload: Vec<RequestData>,
}

impl Request {
    /// Creates a new request, generating a unique id for it
    pub fn new(tenant: impl Into<String>, payload: Vec<RequestData>) -> Self {
        let id = rand::random();
        Self {
            tenant: tenant.into(),
            id,
            payload,
        }
    }

    /// Converts to a string representing the type (or types) contained in the payload
    pub fn to_payload_type_string(&self) -> String {
        self.payload
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>()
            .join(",")
    }
}

/// Represents the payload of a request to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[cfg_attr(feature = "structopt", derive(structopt::StructOpt))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
#[strum(serialize_all = "snake_case")]
pub enum RequestData {
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
        #[cfg_attr(feature = "structopt", structopt(short, long))]
        absolute: bool,

        /// Whether or not to canonicalize the resulting paths, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        ///
        /// Note that the flag absolute must be true to have absolute paths
        /// returned, even if canonicalize is flagged as true
        #[cfg_attr(feature = "structopt", structopt(short, long))]
        canonicalize: bool,

        /// Whether or not to include the root directory in the retrieved
        /// entries
        ///
        /// If included, the root directory will also be a canonicalized,
        /// absolute path and will not follow any of the other flags
        #[cfg_attr(feature = "structopt", structopt(long))]
        include_root: bool,
    },

    /// Creates a directory on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["mkdir"]))]
    DirCreate {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Whether or not to create all parent directories
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
        #[cfg_attr(feature = "structopt", structopt(short, long))]
        canonicalize: bool,

        /// Whether or not to follow symlinks to determine absolute file type (dir/file)
        #[cfg_attr(feature = "structopt", structopt(long))]
        resolve_file_type: bool,
    },

    /// Spawns a new process on the remote machine
    #[cfg_attr(feature = "structopt", structopt(visible_aliases = &["run"]))]
    ProcSpawn {
        /// Name of the command to run
        cmd: String,

        /// Arguments for the command
        args: Vec<String>,

        /// Whether or not the process should be detached, meaning that the process will not be
        /// killed when the associated client disconnects
        #[cfg_attr(feature = "structopt", structopt(long))]
        detached: bool,

        /// If provided, will spawn process in a pty, otherwise spawns directly
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

/// Represents an response to a request performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Response {
    /// A name tied to the requester (tenant)
    pub tenant: String,

    /// A unique id associated with the response
    pub id: usize,

    /// The id of the originating request that yielded this response
    /// (more than one response may have same origin)
    pub origin_id: usize,

    /// The main payload containing a collection of data comprising one or more results
    pub payload: Vec<ResponseData>,
}

impl Response {
    /// Creates a new response, generating a unique id for it
    pub fn new(tenant: impl Into<String>, origin_id: usize, payload: Vec<ResponseData>) -> Self {
        let id = rand::random();
        Self {
            tenant: tenant.into(),
            id,
            origin_id,
            payload,
        }
    }

    /// Converts to a string representing the type (or types) contained in the payload
    pub fn to_payload_type_string(&self) -> String {
        self.payload
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>()
            .join(",")
    }
}

/// Represents the payload of a successful response
#[derive(Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
#[strum(serialize_all = "snake_case")]
pub enum ResponseData {
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
    pub pixel_width: u16,

    /// Height of a cell in pixels. Note that some systems never fill this value and ignore it.
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

    /// Whether or not the process was run in detached mode
    pub detached: bool,

    /// Pty associated with running process if it has one
    pub pty: Option<PtySize>,

    /// Arbitrary id associated with running process
    ///
    /// Not the same as the process' pid!
    pub id: usize,
}

impl From<io::Error> for ResponseData {
    fn from(x: io::Error) -> Self {
        Self::Error(Error::from(x))
    }
}

impl From<walkdir::Error> for ResponseData {
    fn from(x: walkdir::Error) -> Self {
        Self::Error(Error::from(x))
    }
}

impl From<tokio::task::JoinError> for ResponseData {
    fn from(x: tokio::task::JoinError) -> Self {
        Self::Error(Error::from(x))
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
