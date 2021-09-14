/// Capacity associated with a client broadcasting its received messages that
/// do not have a callback associated
pub const CLIENT_BROADCAST_CHANNEL_CAPACITY: usize = 10000;

/// Represents the maximum size (in bytes) that data will be read from pipes
/// per individual `read` call
///
/// Current setting is 16k size
pub const MAX_PIPE_CHUNK_SIZE: usize = 16384;

/// Duration in milliseconds to sleep between reading stdout/stderr chunks
/// to avoid sending many small messages to clients
pub const READ_PAUSE_MILLIS: u64 = 50;

/// Test-only constants
#[cfg(test)]
pub mod test {
    pub const BUFFER_SIZE: usize = 100;
    pub const TENANT: &str = "test-tenant";
}
