use tokio::task::JoinHandle;

/// Interface to engage with a server instance
pub trait ServerRef {
    /// Returns true if the server is no longer running
    fn is_finished(&self) -> bool;

    /// Kills the internal task processing new inbound requests
    fn abort(&self);
}

/// Runtime-specific implementation of [`ServerRef`] for a [`tokio::task::JoinHandle`]
impl ServerRef for JoinHandle<()> {
    fn is_finished(&self) -> bool {
        self.is_finished()
    }

    fn abort(&self) {
        self.abort();
    }
}

mod tcp;
pub use tcp::*;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::*;
