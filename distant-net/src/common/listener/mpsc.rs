use super::Listener;
use async_trait::async_trait;
use derive_more::From;
use std::io;
use tokio::sync::mpsc;

/// Represents a [`Listener`] that uses an [`mpsc::Receiver`] to
/// accept new connections
#[derive(From)]
pub struct MpscListener<T: Send> {
    inner: mpsc::Receiver<T>,
}

impl<T: Send> MpscListener<T> {
    pub fn channel(buffer: usize) -> (mpsc::Sender<T>, Self) {
        let (tx, rx) = mpsc::channel(buffer);
        (tx, Self { inner: rx })
    }
}

#[async_trait]
impl<T: Send> Listener for MpscListener<T> {
    type Output = T;

    async fn accept(&mut self) -> io::Result<Self::Output> {
        self.inner
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
    }
}
