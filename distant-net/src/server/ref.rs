use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::broadcast;
use tokio::task::{JoinError, JoinHandle};

/// Represents a reference to a server
pub struct ServerRef {
    pub(crate) shutdown: broadcast::Sender<()>,
    pub(crate) task: JoinHandle<()>,
}

impl ServerRef {
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown.send(());
    }
}

impl Future for ServerRef {
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
