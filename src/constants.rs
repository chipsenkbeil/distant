use once_cell::sync::Lazy;
use std::{env, path::PathBuf};

/// Represents the maximum size (in bytes) that data will be read from pipes
/// per individual `read` call
///
/// Current setting is 16k size
pub const MAX_PIPE_CHUNK_SIZE: usize = 16384;

/// Capacity associated with a server receiving messages from a connection
/// with a client
pub const SERVER_CONN_MSG_CAPACITY: usize = 10000;

/// Represents maximum time (in milliseconds) to wait on a network request
/// before failing (0 meaning indefinitely)
pub const TIMEOUT: usize = 15000;

/// Duration in milliseconds to sleep between checking for a terminal size change
/// to send resize events to a remote pty
pub const TERMINAL_RESIZE_MILLIS: u64 = 50;

pub static TIMEOUT_STR: Lazy<String> = Lazy::new(|| TIMEOUT.to_string());
pub static SERVER_CONN_MSG_CAPACITY_STR: Lazy<String> =
    Lazy::new(|| SERVER_CONN_MSG_CAPACITY.to_string());

/// Represents the path to the global session file
pub static SESSION_FILE_PATH: Lazy<PathBuf> = Lazy::new(|| env::temp_dir().join("distant.session"));
pub static SESSION_FILE_PATH_STR: Lazy<String> =
    Lazy::new(|| SESSION_FILE_PATH.to_string_lossy().to_string());

/// Represents the path to a socket to communicate instead of a session file
pub static SESSION_SOCKET_PATH: Lazy<PathBuf> = Lazy::new(|| env::temp_dir().join("distant.sock"));
pub static SESSION_SOCKET_PATH_STR: Lazy<String> =
    Lazy::new(|| SESSION_SOCKET_PATH.to_string_lossy().to_string());
