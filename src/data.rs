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

    /// Retrieve a list of all processes being managed by the remote server
    ProcList {},
}
