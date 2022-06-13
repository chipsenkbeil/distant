use crate::Listener;
use async_trait::async_trait;
use derive_more::From;
use std::io;
use tokio::sync::mpsc;

/// Represents a listener used for testing purposes
#[derive(From)]
pub struct TestListener<T: Send> {
    inner: mpsc::Receiver<T>,
}

impl<T: Send> TestListener<T> {
    pub fn channel(buffer: usize) -> (mpsc::Sender<T>, Self) {
        let (tx, rx) = mpsc::channel(buffer);
        (tx, Self { inner: rx })
    }
}

#[async_trait]
impl<T: Send> Listener for TestListener<T> {
    type Output = T;

    async fn accept(&mut self) -> io::Result<Self::Output> {
        self.inner
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
    }
}
