use crate::ServerState;
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::task::{JoinError, JoinHandle};

/// Interface to engage with a server instance
pub trait ServerRef: Send {
    /// Returns a reference to the state of the server
    fn state(&self) -> &ServerState;

    /// Returns true if the server is no longer running
    fn is_finished(&self) -> bool;

    /// Kills the internal task processing new inbound requests
    fn abort(&self);

    fn wait(self) -> Pin<Box<dyn Future<Output = io::Result<()>>>>
    where
        Self: Sized + 'static,
    {
        Box::pin(async {
            let task = tokio::spawn(async move {
                while !self.is_finished() {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            });
            task.await
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
        })
    }
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

    fn wait(self) -> Pin<Box<dyn Future<Output = io::Result<()>>>>
    where
        Self: Sized + 'static,
    {
        Box::pin(async {
            self.task
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
        })
    }
}

impl Future for GenericServerRef {
    type Output = Result<(), JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.task).poll(cx)
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
