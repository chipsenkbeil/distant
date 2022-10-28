use async_trait::async_trait;
use dyn_clone::DynClone;
use std::io;
use tokio::sync::{mpsc, oneshot};

/// Interface representing functionality to shut down an active client.
#[async_trait]
pub trait Shutdown: DynClone {
    /// Attempts to shutdown the client.
    async fn shutdown(&self) -> io::Result<()>;
}

#[async_trait]
impl Shutdown for mpsc::Sender<oneshot::Sender<io::Result<()>>> {
    async fn shutdown(&self) -> io::Result<()> {
        let (tx, rx) = oneshot::channel();
        match self.send(tx).await {
            Ok(_) => match rx.await {
                Ok(x) => x,
                Err(_) => Err(already_shutdown()),
            },
            Err(_) => Err(already_shutdown()),
        }
    }
}

#[inline]
fn already_shutdown() -> io::Error {
    io::Error::new(io::ErrorKind::Other, "Client already shutdown")
}

macro_rules! impl_traits {
    ($($x:tt)+) => {
        impl Clone for Box<dyn $($x)+> {
            fn clone(&self) -> Self {
                dyn_clone::clone_box(&**self)
            }
        }
    };
}

impl_traits!(Shutdown);
impl_traits!(Shutdown + Send);
impl_traits!(Shutdown + Sync);
impl_traits!(Shutdown + Send + Sync);
