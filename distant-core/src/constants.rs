/// Capacity associated stdin, stdout, and stderr pipes receiving data from remote server
pub const CLIENT_PIPE_CAPACITY: usize = 10000;

/// Capacity associated with a client watcher receiving changes
pub const CLIENT_WATCHER_CAPACITY: usize = 100;

/// Capacity associated with a client searcher receiving matches
pub const CLIENT_SEARCHER_CAPACITY: usize = 10000;
