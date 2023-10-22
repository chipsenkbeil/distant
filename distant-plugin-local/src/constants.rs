use std::time::Duration;

/// Capacity associated with the server's file watcher to pass events outbound
pub const SERVER_WATCHER_CAPACITY: usize = 10000;

/// Represents the maximum size (in bytes) that data will be read from pipes
/// per individual `read` call
///
/// Current setting is 16k size
pub const MAX_PIPE_CHUNK_SIZE: usize = 16384;

/// Duration in milliseconds to sleep between reading stdout/stderr chunks
/// to avoid sending many small messages to clients
pub const READ_PAUSE_DURATION: Duration = Duration::from_millis(1);
