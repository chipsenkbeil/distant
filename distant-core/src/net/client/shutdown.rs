use std::future::Future;
use std::io;
use std::pin::Pin;

use dyn_clone::DynClone;
use tokio::sync::{mpsc, oneshot};

/// Interface representing functionality to shut down an active client.
pub trait Shutdown: DynClone + Send + Sync {
    /// Attempts to shutdown the client.
    fn shutdown<'a>(&'a self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;
}

impl Shutdown for mpsc::Sender<oneshot::Sender<io::Result<()>>> {
    fn shutdown<'a>(&'a self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let (tx, rx) = oneshot::channel();
            match self.send(tx).await {
                Ok(_) => match rx.await {
                    Ok(x) => x,
                    Err(_) => Err(already_shutdown()),
                },
                Err(_) => Err(already_shutdown()),
            }
        })
    }
}

#[inline]
fn already_shutdown() -> io::Error {
    io::Error::other("Client already shutdown")
}

impl Clone for Box<dyn Shutdown> {
    fn clone(&self) -> Self {
        dyn_clone::clone_box(&**self)
    }
}
