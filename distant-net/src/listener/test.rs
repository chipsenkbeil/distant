use crate::{Listener, RawTransport};
use async_trait::async_trait;
use derive_more::From;
use std::io;
use tokio::sync::mpsc;

/// Represents a listener used for testing purposes that receives
/// output from a [`mpsc::Receiver`]
#[derive(From)]
pub struct TestListener<T>
where
    T: RawTransport + Send + Sync + 'static,
{
    inner: mpsc::Receiver<T>,
}

impl<T> TestListener<T>
where
    T: RawTransport + Send + Sync + 'static,
{
    pub fn channel(buffer: usize) -> (mpsc::Sender<T>, Self) {
        let (tx, rx) = mpsc::channel(buffer);
        (tx, Self { inner: rx })
    }
}

#[async_trait]
impl<T> Listener for TestListener<T>
where
    T: RawTransport + Send + Sync + 'static,
{
    type Output = T;

    async fn accept(&mut self) -> io::Result<Self::Output> {
        self.inner
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
    }
}
