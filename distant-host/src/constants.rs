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

/// Maximum time to wait for stdout/stderr to drain after a process exits.
/// Guards against Windows ConPTY readers that may never signal EOF.
pub const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum time to wait for a process to report completion after being killed.
/// Used in tests to catch regressions where killed processes block on orphaned
/// child process pipes (e.g. Windows `cmd /C` → child process trees).
pub const KILL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
