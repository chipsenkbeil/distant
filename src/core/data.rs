use derive_more::IsVariant;
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};
use structopt::StructOpt;
use strum::AsRefStr;

/// Represents the request to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Request {
    /// A name tied to the requester (tenant)
    pub tenant: String,

    /// A unique id associated with the request
    pub id: usize,

    /// The main payload containing the type and data of the request
    pub payload: RequestPayload,
}

impl Request {
    /// Creates a new request, generating a unique id for it
    pub fn new(tenant: impl Into<String>, payload: RequestPayload) -> Self {
        let id = rand::random();
        Self {
            tenant: tenant.into(),
            id,
            payload,
        }
    }
}

/// Represents the payload of a request to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, StructOpt, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    deny_unknown_fields,
    tag = "type",
    content = "data"
)]
#[strum(serialize_all = "snake_case")]
pub enum RequestPayload {
    /// Reads a file from the specified path on the remote machine
    #[structopt(visible_aliases = &["cat"])]
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
        data: Vec<u8>,
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
        data: Vec<u8>,
    },

    /// Appends text to a file, creating it if it does not exist, on the remote machine
    FileAppendText {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Data for server-side writing of content
        text: String,
    },

    /// Reads a directory from the specified path on the remote machine
    #[structopt(visible_aliases = &["ls"])]
    DirRead {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Maximum depth to traverse with 0 indicating there is no maximum
        /// depth and 1 indicating the most immediate children within the
        /// directory
        #[serde(default = "one")]
        #[structopt(short, long, default_value = "1")]
        depth: usize,

        /// Whether or not to return absolute or relative paths
        #[structopt(short, long)]
        absolute: bool,

        /// Whether or not to canonicalize the resulting paths, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        ///
        /// Note that the flag absolute must be true to have absolute paths
        /// returned, even if canonicalize is flagged as true
        #[structopt(short, long)]
        canonicalize: bool,

        /// Whether or not to include the root directory in the retrieved
        /// entries
        ///
        /// If included, the root directory will also be a canonicalized,
        /// absolute path and will not follow any of the other flags
        #[structopt(long)]
        include_root: bool,
    },

    /// Creates a directory on the remote machine
    #[structopt(visible_aliases = &["mkdir"])]
    DirCreate {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Whether or not to create all parent directories
        #[structopt(short, long)]
        all: bool,
    },

    /// Removes a file or directory on the remote machine
    #[structopt(visible_aliases = &["rm"])]
    Remove {
        /// The path to the file or directory on the remote machine
        path: PathBuf,

        /// Whether or not to remove all contents within directory if is a directory.
        /// Does nothing different for files
        #[structopt(short, long)]
        force: bool,
    },

    /// Copies a file or directory on the remote machine
    #[structopt(visible_aliases = &["cp"])]
    Copy {
        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for copy of file or directory
        dst: PathBuf,
    },

    /// Moves/renames a file or directory on the remote machine
    #[structopt(visible_aliases = &["mv"])]
    Rename {
        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for the file or directory
        dst: PathBuf,
    },

    /// Retrieves filesystem metadata for the specified path on the remote machine
    Metadata {
        /// The path to the file, directory, or symlink on the remote machine
        path: PathBuf,

        /// Whether or not to include a canonicalized version of the path, meaning
        /// returning the canonical, absolute form of a path with all
        /// intermediate components normalized and symbolic links resolved
        #[structopt(short, long)]
        canonicalize: bool,
    },

    /// Runs a process on the remote machine
    #[structopt(visible_aliases = &["run"])]
    ProcRun {
        /// Name of the command to run
        cmd: String,

        /// Arguments for the command
        args: Vec<String>,
    },

    /// Kills a process running on the remote machine
    #[structopt(visible_aliases = &["kill"])]
    ProcKill {
        /// Id of the actively-running process
        id: usize,
    },

    /// Sends additional data to stdin of running process
    ProcStdin {
        /// Id of the actively-running process to send stdin data
        id: usize,

        /// Data to send to a process's stdin pipe
        data: String,
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

    /// The id of the originating request, if there was one
    /// (some responses are sent unprompted)
    pub origin_id: Option<usize>,

    /// The main payload containing the type and data of the response
    pub payload: ResponsePayload,
}

impl Response {
    /// Creates a new response, generating a unique id for it
    pub fn new(
        tenant: impl Into<String>,
        origin_id: Option<usize>,
        payload: ResponsePayload,
    ) -> Self {
        let id = rand::random();
        Self {
            tenant: tenant.into(),
            id,
            origin_id,
            payload,
        }
    }
}

/// Represents the payload of a successful response
#[derive(Clone, Debug, PartialEq, Eq, AsRefStr, IsVariant, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    deny_unknown_fields,
    tag = "type",
    content = "data"
)]
#[strum(serialize_all = "snake_case")]
pub enum ResponsePayload {
    /// General okay with no extra data, returned in cases like
    /// creating or removing a directory, copying a file, or renaming
    /// a file
    Ok,

    /// General-purpose failure that occurred from some request
    Error {
        /// Details about the error
        description: String,
    },

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

    /// Represents metadata about some filesystem object (file, directory, symlink) on remote machine
    Metadata {
        /// Canonicalized path to the file or directory, resolving symlinks, only included
        /// if flagged during the request
        canonicalized_path: Option<PathBuf>,

        /// Represents the type of the entry as a file/dir/symlink
        file_type: FileType,

        /// Size of the file/directory/symlink in bytes
        len: u64,

        /// Whether or not the file/directory/symlink is marked as unwriteable
        readonly: bool,

        /// Represents the last time (in milliseconds) when the file/directory/symlink was accessed;
        /// can be optional as certain systems don't support this
        accessed: Option<u128>,

        /// Represents when (in milliseconds) the file/directory/symlink was created;
        /// can be optional as certain systems don't support this
        created: Option<u128>,

        /// Represents the last time (in milliseconds) when the file/directory/symlink was modified;
        /// can be optional as certain systems don't support this
        modified: Option<u128>,
    },

    /// Response to starting a new process
    ProcStart {
        /// Arbitrary id associated with running process
        id: usize,
    },

    /// Actively-transmitted stdout as part of running process
    ProcStdout {
        /// Arbitrary id associated with running process
        id: usize,

        /// Data read from a process' stdout pipe
        data: String,
    },

    /// Actively-transmitted stderr as part of running process
    ProcStderr {
        /// Arbitrary id associated with running process
        id: usize,

        /// Data read from a process' stderr pipe
        data: String,
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
    SystemInfo {
        /// Family of the operating system as described in
        /// https://doc.rust-lang.org/std/env/consts/constant.FAMILY.html
        family: String,

        /// Name of the specific operating system as described in
        /// https://doc.rust-lang.org/std/env/consts/constant.OS.html
        os: String,

        /// Architecture of the CPI as described in
        /// https://doc.rust-lang.org/std/env/consts/constant.ARCH.html
        arch: String,

        /// Current working directory of the running server process
        current_dir: PathBuf,

        /// Primary separator for path components for the current platform
        /// as defined in https://doc.rust-lang.org/std/path/constant.MAIN_SEPARATOR.html
        main_separator: char,
    },
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
    SymLink,
}

/// Represents information about a running process
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct RunningProcess {
    /// Name of the command being run
    pub cmd: String,

    /// Arguments for the command
    pub args: Vec<String>,

    /// Arbitrary id associated with running process
    ///
    /// Not the same as the process' pid!
    pub id: usize,
}

/// General purpose error type that can be sent across the wire
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Error {
    /// Label describing the kind of error
    pub kind: String,

    /// Description of the error itself
    pub description: String,
}

impl From<io::Error> for Error {
    fn from(x: io::Error) -> Self {
        Self {
            kind: format!("{:?}", x.kind()),
            description: format!("{}", x),
        }
    }
}

impl From<walkdir::Error> for Error {
    fn from(x: walkdir::Error) -> Self {
        if x.io_error().is_some() {
            x.into_io_error().map(Self::from).unwrap()
        } else {
            Self {
                kind: String::from("Loop"),
                description: format!("{}", x),
            }
        }
    }
}

/// Used to provide a default serde value of 1
const fn one() -> usize {
    1
}
