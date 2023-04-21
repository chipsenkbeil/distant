use derive_more::{From, IsVariant};
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};
use strum::{AsRefStr, EnumDiscriminants, EnumIter, EnumMessage, EnumString};

mod capabilities;
pub use capabilities::*;

mod change;
pub use change::*;

mod cmd;
pub use cmd::*;

mod error;
pub use error::*;

mod filesystem;
pub use filesystem::*;

mod metadata;
pub use metadata::*;

mod pty;
pub use pty::*;

mod search;
pub use search::*;

mod system;
pub use system::*;

mod utils;
pub(crate) use utils::*;

/// Id for a remote process
pub type ProcessId = u32;

/// Mapping of environment variables
pub type Environment = distant_net::common::Map;

/// Represents a wrapper around a distant message, supporting single and batch requests
#[derive(Clone, Debug, From, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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

    /// Returns reference to single value if msg is single variant
    pub fn as_single(&self) -> Option<&T> {
        match self {
            Self::Single(x) => Some(x),
            _ => None,
        }
    }

    /// Returns mutable reference to single value if msg is single variant
    pub fn as_mut_single(&mut self) -> Option<&T> {
        match self {
            Self::Single(x) => Some(x),
            _ => None,
        }
    }

    /// Returns the single value if msg is single variant
    pub fn into_single(self) -> Option<T> {
        match self {
            Self::Single(x) => Some(x),
            _ => None,
        }
    }

    /// Returns true if msg has a batch of payloads
    pub fn is_batch(&self) -> bool {
        matches!(self, Self::Batch(_))
    }

    /// Returns reference to batch value if msg is batch variant
    pub fn as_batch(&self) -> Option<&[T]> {
        match self {
            Self::Batch(x) => Some(x),
            _ => None,
        }
    }

    /// Returns mutable reference to batch value if msg is batch variant
    pub fn as_mut_batch(&mut self) -> Option<&mut [T]> {
        match self {
            Self::Batch(x) => Some(x),
            _ => None,
        }
    }

    /// Returns the batch value if msg is batch variant
    pub fn into_batch(self) -> Option<Vec<T>> {
        match self {
            Self::Batch(x) => Some(x),
            _ => None,
        }
    }

    /// Convert into a collection of payload data
    pub fn into_vec(self) -> Vec<T> {
        match self {
            Self::Single(x) => vec![x],
            Self::Batch(x) => x,
        }
    }
}

#[cfg(feature = "schemars")]
impl<T: schemars::JsonSchema> DistantMsg<T> {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(DistantMsg<T>)
    }
}

/// Represents the payload of a request to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, EnumDiscriminants, IsVariant, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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
#[cfg_attr(
    feature = "schemars",
    strum_discriminants(derive(schemars::JsonSchema))
)]
#[strum_discriminants(name(CapabilityKind))]
#[strum_discriminants(strum(serialize_all = "snake_case"))]
#[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
pub enum DistantRequestData {
    /// Retrieve information about the server's capabilities
    #[strum_discriminants(strum(message = "Supports retrieving capabilities"))]
    Capabilities {},

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
        #[cfg_attr(feature = "schemars", schemars(with = "Vec<u8>"))]
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
        #[cfg_attr(feature = "schemars", schemars(with = "Vec<u8>"))]
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
        #[serde(default = "one")]
        depth: usize,

        /// Whether or not to return absolute or relative paths
        #[serde(default)]
        absolute: bool,

        /// Whether or not to canonicalize the resulting paths, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        ///
        /// Note that the flag absolute must be true to have absolute paths
        /// returned, even if canonicalize is flagged as true
        #[serde(default)]
        canonicalize: bool,

        /// Whether or not to include the root directory in the retrieved
        /// entries
        ///
        /// If included, the root directory will also be a canonicalized,
        /// absolute path and will not follow any of the other flags
        #[serde(default)]
        include_root: bool,
    },

    /// Creates a directory on the remote machine
    #[strum_discriminants(strum(message = "Supports creating directory"))]
    DirCreate {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Whether or not to create all parent directories
        #[serde(default)]
        all: bool,
    },

    /// Removes a file or directory on the remote machine
    #[strum_discriminants(strum(message = "Supports removing files, directories, and symlinks"))]
    Remove {
        /// The path to the file or directory on the remote machine
        path: PathBuf,

        /// Whether or not to remove all contents within directory if is a directory.
        /// Does nothing different for files
        #[serde(default)]
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
        #[serde(default)]
        recursive: bool,

        /// Filter to only report back specified changes
        #[serde(default)]
        only: Vec<ChangeKind>,

        /// Filter to report back changes except these specified changes
        #[serde(default)]
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
        #[serde(default)]
        canonicalize: bool,

        /// Whether or not to follow symlinks to determine absolute file type (dir/file)
        #[serde(default)]
        resolve_file_type: bool,
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
        #[serde(default)]
        environment: Environment,

        /// Alternative current directory for the remote process
        #[serde(default)]
        current_dir: Option<PathBuf>,

        /// If provided, will spawn process in a pty, otherwise spawns directly
        #[serde(default)]
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
        #[cfg_attr(feature = "schemars", schemars(with = "Vec<u8>"))]
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
}

#[cfg(feature = "schemars")]
impl DistantRequestData {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(DistantRequestData)
    }
}

/// Represents the payload of a successful response
#[derive(Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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
        #[serde(with = "serde_bytes")]
        #[cfg_attr(feature = "schemars", schemars(with = "Vec<u8>"))]
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
        #[cfg_attr(feature = "schemars", schemars(with = "Vec<u8>"))]
        data: Vec<u8>,
    },

    /// Actively-transmitted stderr as part of running process
    ProcStderr {
        /// Arbitrary id associated with running process
        id: ProcessId,

        /// Data read from a process' stderr pipe
        #[serde(with = "serde_bytes")]
        #[cfg_attr(feature = "schemars", schemars(with = "Vec<u8>"))]
        data: Vec<u8>,
    },

    /// Response to a process finishing
    ProcDone {
        /// Arbitrary id associated with running process
        id: ProcessId,

        /// Whether or not termination was successful
        success: bool,

        /// Exit code associated with termination, will be missing if terminated by signal
        code: Option<i32>,
    },

    /// Response to retrieving information about the server and the system it is on
    SystemInfo(SystemInfo),

    /// Response to retrieving information about the server's capabilities
    Capabilities { supported: Capabilities },
}

#[cfg(feature = "schemars")]
impl DistantResponseData {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(DistantResponseData)
    }
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
