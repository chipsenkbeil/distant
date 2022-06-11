/// Capacity associated with a client mailboxes for receiving multiple responses to a request
pub const CLIENT_MAILBOX_CAPACITY: usize = 10000;

/// Capacity associated stdin, stdout, and stderr pipes receiving data from remote server
pub const CLIENT_PIPE_CAPACITY: usize = 10000;

/// Capacity associated with a client watcher receiving changes
pub const CLIENT_WATCHER_CAPACITY: usize = 100;

/// Capacity associated with the server's file watcher to pass events outbound
pub const SERVER_WATCHER_CAPACITY: usize = 10000;

/// Represents the maximum size (in bytes) that data will be read from pipes
/// per individual `read` call
///
/// Current setting is 16k size
pub const MAX_PIPE_CHUNK_SIZE: usize = 16384;

/// Duration in milliseconds to sleep between reading stdout/stderr chunks
/// to avoid sending many small messages to clients
pub const READ_PAUSE_MILLIS: u64 = 50;

/// Maximum message capacity per connection for the distant server
pub const MAX_MSG_CAPACITY: usize = 10000;
