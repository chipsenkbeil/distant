use crate::ServerState;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// Interface to engage with a server instance
pub trait ServerRef {
    /// Returns a reference to the state of the server
    fn state(&self) -> &ServerState;

    /// Returns true if the server is no longer running
    fn is_finished(&self) -> bool;

    /// Kills the internal task processing new inbound requests
    fn abort(&self);
}

/// Represents a generic reference to a server
pub struct GenericServerRef {
    pub(crate) state: Arc<ServerState>,
    pub(crate) task: JoinHandle<()>,
}

/// Runtime-specific implementation of [`ServerRef`] for a [`tokio::task::JoinHandle`]
impl ServerRef for GenericServerRef {
    fn state(&self) -> &ServerState {
        &self.state
    }

    fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    fn abort(&self) {
        self.task.abort();
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
