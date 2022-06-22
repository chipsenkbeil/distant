use derive_more::{From, IsVariant};
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};
use strum::AsRefStr;

#[cfg(feature = "clap")]
use strum::VariantNames;

mod change;
pub use change::*;

#[cfg(feature = "clap")]
mod clap_impl;

mod error;
pub use error::*;

mod filesystem;
pub use filesystem::*;

mod metadata;
pub use metadata::*;

mod pty;
pub use pty::*;

mod system;
pub use system::*;

mod utils;
pub(crate) use utils::*;

/// Type alias for a vec of bytes
///
/// NOTE: This only exists to support properly parsing a Vec<u8> from an entire string
///       with clap rather than trying to parse a string as a singular u8
pub type ByteVec = Vec<u8>;

#[cfg(feature = "clap")]
fn parse_byte_vec(src: &str) -> ByteVec {
    src.as_bytes().to_vec()
}

/// Represents a wrapper around a distant message, supporting single and batch requests
#[derive(Clone, Debug, From, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DistantMsg<T> {
    Single(T),
    Batch(Vec<T>),
}

impl<T> DistantMsg<T> {
    /// Returns true if msg has a single payload
    pub fn is_single(&self) -> bool {
        matches!(self, Self::Single(_))
    }

    /// Returns true if msg has a batch of payloads
    pub fn is_batch(&self) -> bool {
        matches!(self, Self::Batch(_))
    }

    /// Convert into a collection of payload data
    pub fn into_vec(self) -> Vec<T> {
        match self {
            Self::Single(x) => vec![x],
            Self::Batch(x) => x,
        }
    }
}

/// Represents the payload of a request to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, IsVariant, Serialize, Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::Subcommand))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
#[cfg_attr(feature = "clap", clap(rename_all = "kebab-case"))]
pub enum DistantRequestData {
    /// Reads a file from the specified path on the remote machine
    #[cfg_attr(feature = "clap", clap(visible_aliases = &["cat"]))]
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
        #[cfg_attr(feature = "clap", clap(parse(from_str = parse_byte_vec)))]
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
        #[cfg_attr(feature = "clap", clap(parse(from_str = parse_byte_vec)))]
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
    #[cfg_attr(feature = "clap", clap(visible_aliases = &["ls"]))]
    DirRead {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Maximum depth to traverse with 0 indicating there is no maximum
        /// depth and 1 indicating the most immediate children within the
        /// directory
        #[serde(default = "one")]
        #[cfg_attr(feature = "clap", clap(short, long, default_value = "1"))]
        depth: usize,

        /// Whether or not to return absolute or relative paths
        #[serde(default)]
        #[cfg_attr(feature = "clap", clap(short, long))]
        absolute: bool,

        /// Whether or not to canonicalize the resulting paths, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        ///
        /// Note that the flag absolute must be true to have absolute paths
        /// returned, even if canonicalize is flagged as true
        #[serde(default)]
        #[cfg_attr(feature = "clap", clap(short, long))]
        canonicalize: bool,

        /// Whether or not to include the root directory in the retrieved
        /// entries
        ///
        /// If included, the root directory will also be a canonicalized,
        /// absolute path and will not follow any of the other flags
        #[serde(default)]
        #[cfg_attr(feature = "clap", clap(long))]
        include_root: bool,
    },

    /// Creates a directory on the remote machine
    #[cfg_attr(feature = "clap", clap(visible_aliases = &["mkdir"]))]
    DirCreate {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Whether or not to create all parent directories
        #[serde(default)]
        #[cfg_attr(feature = "clap", clap(short, long))]
        all: bool,
    },

    /// Removes a file or directory on the remote machine
    #[cfg_attr(feature = "clap", clap(visible_aliases = &["rm"]))]
    Remove {
        /// The path to the file or directory on the remote machine
        path: PathBuf,

        /// Whether or not to remove all contents within directory if is a directory.
        /// Does nothing different for files
        #[serde(default)]
        #[cfg_attr(feature = "clap", clap(short, long))]
        force: bool,
    },

    /// Copies a file or directory on the remote machine
    #[cfg_attr(feature = "clap", clap(visible_aliases = &["cp"]))]
    Copy {
        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for copy of file or directory
        dst: PathBuf,
    },

    /// Moves/renames a file or directory on the remote machine
    #[cfg_attr(feature = "clap", clap(visible_aliases = &["mv"]))]
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
        #[cfg_attr(feature = "clap", clap(short, long))]
        recursive: bool,

        /// Filter to only report back specified changes
        #[serde(default)]
        #[cfg_attr(
            feature = "clap",
            clap(short, long, possible_values = ChangeKind::VARIANTS)
        )]
        only: Vec<ChangeKind>,

        /// Filter to report back changes except these specified changes
        #[serde(default)]
        #[cfg_attr(
            feature = "clap", 
            clap(short, long, possible_values = ChangeKind::VARIANTS)
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
        #[cfg_attr(feature = "clap", clap(short, long))]
        canonicalize: bool,

        /// Whether or not to follow symlinks to determine absolute file type (dir/file)
        #[serde(default)]
        #[cfg_attr(feature = "clap", clap(long))]
        resolve_file_type: bool,
    },

    /// Spawns a new process on the remote machine
    #[cfg_attr(feature = "clap", clap(visible_aliases = &["spawn", "run"]))]
    ProcSpawn {
        /// The full command to run including arguments
        cmd: String,

        /// Whether or not the process should be persistent, meaning that the process will not be
        /// killed when the associated client disconnects
        #[serde(default)]
        #[cfg_attr(feature = "clap", clap(long))]
        persist: bool,

        /// If provided, will spawn process in a pty, otherwise spawns directly
        #[serde(default)]
        #[cfg_attr(feature = "clap", clap(long))]
        pty: Option<PtySize>,
    },

    /// Kills a process running on the remote machine
    #[cfg_attr(feature = "clap", clap(visible_aliases = &["kill"]))]
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

    /// Response to retrieving information about the server and the system it is on
    SystemInfo(SystemInfo),
}

impl From<io::Error> for DistantResponseData {
    fn from(x: io::Error) -> Self {
        Self::Error(Error::from(x))
    }
}

/// Used to provide a default serde value of 1
const fn one() -> usize {
    1
}
