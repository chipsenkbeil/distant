use std::{env, path::PathBuf};

/// Capacity associated with a client broadcasting its received messages that
/// do not have a callback associated
pub const CLIENT_BROADCAST_CHANNEL_CAPACITY: usize = 100;

/// Represents the maximum size (in bytes) that data will be read from pipes
/// per individual `read` call
///
/// Current setting is 1k size
pub const MAX_PIPE_CHUNK_SIZE: usize = 1024;

/// Represents the length of the salt to use for encryption
pub const SALT_LEN: usize = 16;

lazy_static::lazy_static! {
    /// Represents the path to the global session file
    pub static ref SESSION_FILE_PATH: PathBuf = env::temp_dir().join("distant.session");
    pub static ref SESSION_FILE_PATH_STR: String = SESSION_FILE_PATH.to_string_lossy().to_string();

    /// Represents the path to a socket to communicate instead of a session file
    pub static ref SESSION_SOCKET_PATH: PathBuf = env::temp_dir().join("distant.sock");
    pub static ref SESSION_SOCKET_PATH_STR: String = SESSION_SOCKET_PATH.to_string_lossy().to_string();
}
