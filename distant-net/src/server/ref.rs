use super::ServerState;
use crate::common::AsAny;
use log::*;
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
pub trait ServerRef: AsAny + Send {
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

impl dyn ServerRef {
    /// Attempts to convert this ref into a concrete ref by downcasting
    pub fn as_server_ref<R: ServerRef>(&self) -> Option<&R> {
        self.as_any().downcast_ref::<R>()
    }

    /// Attempts to convert this mutable ref into a concrete mutable ref by downcasting
    pub fn as_mut_server_ref<R: ServerRef>(&mut self) -> Option<&mut R> {
        self.as_mut_any().downcast_mut::<R>()
    }

    /// Attempts to convert this into a concrete, boxed ref by downcasting
    pub fn into_boxed_server_ref<R: ServerRef>(
        self: Box<Self>,
    ) -> Result<Box<R>, Box<dyn std::any::Any>> {
        self.into_any().downcast::<R>()
    }

    /// Waits for the server to complete by continuously polling the finished state.
    pub async fn polling_wait(&self) -> io::Result<()> {
        while !self.is_finished() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }
}

/// Represents a generic reference to a server
pub struct GenericServerRef {
    pub(crate) state: Arc<ServerState>,
    pub(crate) task: JoinHandle<()>,
}

/// Runtime-specific implementation of [`ServerRef`] for a [`tokio::task::JoinHandle`]
impl ServerRef for GenericServerRef {
    fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    fn abort(&self) {
        self.task.abort();

        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            for (id, connection) in state.connections.read().await.iter() {
                debug!("Aborting connection {}", id);
                connection.abort();
            }
        });
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
