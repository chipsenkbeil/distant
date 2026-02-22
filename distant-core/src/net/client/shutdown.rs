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

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test(tokio::test)]
    async fn shutdown_should_succeed_when_receiver_responds_with_ok() {
        let (tx, mut rx) = mpsc::channel::<oneshot::Sender<io::Result<()>>>(1);

        // Spawn a handler that receives the oneshot sender and responds with Ok
        tokio::spawn(async move {
            if let Some(responder) = rx.recv().await {
                responder.send(Ok(())).ok();
            }
        });

        let result = tx.shutdown().await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn shutdown_should_propagate_error_from_receiver() {
        let (tx, mut rx) = mpsc::channel::<oneshot::Sender<io::Result<()>>>(1);

        // Handler responds with an error
        tokio::spawn(async move {
            if let Some(responder) = rx.recv().await {
                responder.send(Err(io::Error::other("custom error"))).ok();
            }
        });

        let result = tx.shutdown().await;
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains("custom error"));
    }

    #[test(tokio::test)]
    async fn shutdown_should_fail_with_already_shutdown_when_receiver_dropped() {
        let (tx, rx) = mpsc::channel::<oneshot::Sender<io::Result<()>>>(1);
        drop(rx);

        let err = tx.shutdown().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains("already shutdown"));
    }

    #[test(tokio::test)]
    async fn shutdown_should_fail_with_already_shutdown_when_oneshot_responder_dropped() {
        let (tx, mut rx) = mpsc::channel::<oneshot::Sender<io::Result<()>>>(1);

        // Handler receives the oneshot sender but drops it without responding
        tokio::spawn(async move {
            if let Some(_responder) = rx.recv().await {
                // Drop the responder without sending anything
            }
        });

        let err = tx.shutdown().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(err.to_string().contains("already shutdown"));
    }

    #[test(tokio::test)]
    async fn boxed_shutdown_should_be_cloneable() {
        let (tx, _rx) = mpsc::channel::<oneshot::Sender<io::Result<()>>>(1);
        let boxed: Box<dyn Shutdown> = Box::new(tx);
        let _cloned = boxed.clone();
    }
}
