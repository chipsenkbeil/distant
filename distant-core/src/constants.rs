/// Capacity associated stdin, stdout, and stderr pipes receiving data from remote server
pub const CLIENT_PIPE_CAPACITY: usize = 10000;

/// Capacity associated with a client watcher receiving changes
pub const CLIENT_WATCHER_CAPACITY: usize = 100;

/// Capacity associated with a client searcher receiving matches
pub const CLIENT_SEARCHER_CAPACITY: usize = 10000;

/// Capacity associated with a client tunnel receiving data
pub const CLIENT_TUNNEL_CAPACITY: usize = 10000;

/// Buffer size for reading from tunnel relay connections (SSH channels, TCP streams).
pub const TUNNEL_RELAY_BUFFER_SIZE: usize = 8192;

/// Channel capacity for queuing tunnel write data before backpressure.
pub const TUNNEL_CHANNEL_CAPACITY: usize = 1024;

/// Channel capacity for the SSH launch tunnel's `InmemoryTransport`.
pub const TUNNEL_TRANSPORT_CAPACITY: usize = 100;
