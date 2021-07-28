use derive_more::IsVariant;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use structopt::StructOpt;
use strum::AsRefStr;

/// Represents the request to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Request {
    /// A unique id associated with the request
    pub id: usize,

    /// The main payload containing the type and data of the request
    pub payload: RequestPayload,
}

impl From<RequestPayload> for Request {
    /// Produces a new request with the given payload and a randomly-generated id
    fn from(payload: RequestPayload) -> Self {
        let id = rand::random();
        Self { id, payload }
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

        /// Source for client-side loading of content (if not provided, stdin is used)
        #[serde(skip)]
        input: Option<PathBuf>,

        /// Data for server-side writing of content
        #[structopt(skip)]
        data: Vec<u8>,
    },

    /// Appends to a file, creating it if it does not exist, on the remote machine
    FileAppend {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Source for client-side loading of content (if not provided, stdin is used)
        #[serde(skip)]
        input: Option<PathBuf>,

        /// Data for server-side writing of content
        #[structopt(skip)]
        data: Vec<u8>,
    },

    /// Reads a directory from the specified path on the remote machine
    #[structopt(visible_aliases = &["ls"])]
    DirRead {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Whether or not to read subdirectories recursively
        #[structopt(short, long)]
        all: bool,
    },

    /// Creates a directory on the remote machine
    DirCreate {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Whether or not to create all parent directories
        #[structopt(short, long)]
        all: bool,
    },

    /// Removes a file or directory on the remote machine
    Remove {
        /// The path to the file or directory on the remote machine
        path: PathBuf,

        /// Whether or not to remove all contents within directory if is a directory.
        /// Does nothing different for files
        #[structopt(short, long)]
        force: bool,
    },

    /// Copies a file or directory on the remote machine
    Copy {
        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for copy of file or directory
        dst: PathBuf,
    },

    /// Moves/renames a file or directory on the remote machine
    Rename {
        /// The path to the file or directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for the file or directory
        dst: PathBuf,
    },

    /// Runs a process on the remote machine
    ProcRun {
        /// Name of the command to run
        cmd: String,

        /// Arguments for the command
        args: Vec<String>,

        /// Whether or not to detach from the running process without killing it
        #[structopt(long)]
        detach: bool,
    },

    /// Re-connects to a detached process on the remote machine (to receive stdout/stderr)
    ProcConnect {
        /// Id of the actively-running process
        id: usize,
    },

    /// Kills a process running on the remote machine
    ProcKill {
        /// Id of the actively-running process
        id: usize,
    },

    /// Sends additional data to stdin of running process
    ProcStdin {
        /// Id of the actively-running process to send stdin data
        id: usize,

        /// Data to send to stdin of process
        data: Vec<u8>,
    },

    /// Retrieve a list of all processes being managed by the remote server
    ProcList {},
}

/// Represents an response to a request performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct Response {
    /// A unique id associated with the response
    pub id: usize,

    /// The id of the originating request, if there was one
    /// (some responses are sent unprompted)
    pub origin_id: Option<usize>,

    /// The main payload containing the type and data of the response
    pub payload: ResponsePayload,
}

impl Response {
    /// Produces a new response with the given payload and origin id while supplying
    /// randomly-generated id
    pub fn from_payload_with_origin(payload: ResponsePayload, origin_id: usize) -> Self {
        let id = rand::random();
        Self {
            id,
            origin_id: Some(origin_id),
            payload,
        }
    }
}

impl From<ResponsePayload> for Response {
    /// Produces a new response with the given payload, no origin id, and a randomly-generated id
    fn from(payload: ResponsePayload) -> Self {
        let id = rand::random();
        Self {
            id,
            origin_id: None,
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

        /// Data sent to stdout by process
        data: Vec<u8>,
    },

    /// Actively-transmitted stderr as part of running process
    ProcStderr {
        /// Arbitrary id associated with running process
        id: usize,

        /// Data sent to stderr by process
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, IsVariant, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields, untagged)]
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
