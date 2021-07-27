use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use structopt::StructOpt;

/// Represents an operation to be performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, StructOpt, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    deny_unknown_fields,
    tag = "type",
    content = "payload"
)]
pub enum Operation {
    /// Reads a file from the specified path on the remote machine
    #[structopt(visible_aliases = &["cat"])]
    FileRead {
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

    /// Removes a directory on the remote machine
    DirRemove {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Whether or not to remove all contents within directory; if false
        /// and there are still contents, then the directory is not removed
        #[structopt(short, long)]
        all: bool,
    },

    /// Copies a file/directory on the remote machine
    Copy {
        /// The path to the file/directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for copy of file/directory
        dst: PathBuf,
    },

    /// Moves/renames a file or directory on the remote machine
    Rename {
        /// The path to the file/directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for the file/directory
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

/// Represents an response to an operation performed on the remote machine
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    deny_unknown_fields,
    tag = "status",
    content = "payload"
)]
pub enum Response {
    /// Represents a successfully-handled operation
    Ok(ResponsePayload),

    /// Represents an operation that failed
    Error {
        /// The message associated with the failure
        msg: String,
    },
}

/// Represents the payload of a successful response
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "snake_case",
    deny_unknown_fields,
    tag = "type",
    content = "data"
)]
pub enum ResponsePayload {
    /// Response to reading a file
    FileRead {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Contents of the file
        data: Vec<u8>,
    },

    /// Response to writing a file
    FileWrite {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Total bytes written
        bytes_written: usize,
    },

    /// Response to appending to a file
    FileAppend {
        /// The path to the file on the remote machine
        path: PathBuf,

        /// Total bytes written
        bytes_written: usize,
    },

    /// Response to reading a directory
    DirRead {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Entries contained within directory
        entries: Vec<DirEntry>,
    },

    /// Response to creating a directory
    DirCreate {
        /// The path to the directory on the remote machine
        path: PathBuf,
    },

    /// Response to removing a directory
    DirRemove {
        /// The path to the directory on the remote machine
        path: PathBuf,

        /// Total files & directories removed within the directory (0 if directory was empty)
        total_removed: usize,
    },

    /// Response to copying a file/directory
    Copy {
        /// The path to the file/directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for copy of file/directory
        dst: PathBuf,
    },

    /// Response to moving/renaming a file/directory
    Rename {
        /// The path to the file/directory on the remote machine
        src: PathBuf,

        /// New location on the remote machine for the file/directory
        dst: PathBuf,
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
    ProcList {
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
