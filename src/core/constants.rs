use std::{env, path::PathBuf};

/// Represents maximum time (in milliseconds) to wait on a network request
/// before failing (0 meaning indefinitely)
pub const TIMEOUT: usize = 15000;

/// Capacity associated with a client broadcasting its received messages that
/// do not have a callback associated
pub const CLIENT_BROADCAST_CHANNEL_CAPACITY: usize = 10000;

/// Capacity associated with a server receiving messages from a connection
/// with a client
pub const SERVER_CONN_MSG_CAPACITY: usize = 10000;

/// Represents the maximum size (in bytes) that data will be read from pipes
/// per individual `read` call
///
/// Current setting is 16k size
pub const MAX_PIPE_CHUNK_SIZE: usize = 16384;

/// Duration in milliseconds to sleep between reading stdout/stderr chunks
/// to avoid sending many small messages to clients
pub const READ_PAUSE_MILLIS: u64 = 50;

/// Represents the length of the salt to use for encryption
pub const SALT_LEN: usize = 16;

/// Test-only constants
#[cfg(test)]
pub mod test {
    pub const BUFFER_SIZE: usize = 100;
    pub const TENANT: &str = "test-tenant";
}

lazy_static::lazy_static! {
    pub static ref TIMEOUT_STR: String = TIMEOUT.to_string();
    pub static ref SERVER_CONN_MSG_CAPACITY_STR: String = SERVER_CONN_MSG_CAPACITY.to_string();

    /// Represents the path to the global session file
    pub static ref SESSION_FILE_PATH: PathBuf = env::temp_dir().join("distant.session");
    pub static ref SESSION_FILE_PATH_STR: String = SESSION_FILE_PATH.to_string_lossy().to_string();

    /// Represents the path to a socket to communicate instead of a session file
    pub static ref SESSION_SOCKET_PATH: PathBuf = env::temp_dir().join("distant.sock");
    pub static ref SESSION_SOCKET_PATH_STR: String = SESSION_SOCKET_PATH.to_string_lossy().to_string();
}
