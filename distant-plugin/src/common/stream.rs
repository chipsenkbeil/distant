use async_trait::async_trait;
use tokio::sync::mpsc;

/// Interface to an asynchronous stream of items.
#[async_trait]
pub trait Stream: Send {
    type Item: Send;

    /// Retrieves the next item from the stream, returning `None` if no more items are available
    /// from the stream.
    async fn next(&mut self) -> Option<Self::Item>;
}

#[async_trait]
impl<T: Send> Stream for mpsc::UnboundedReceiver<T> {
    type Item = T;

    async fn next(&mut self) -> Option<Self::Item> {
        self.recv().await
    }
}

#[async_trait]
impl<T: Send> Stream for mpsc::Receiver<T> {
    type Item = T;

    async fn next(&mut self) -> Option<Self::Item> {
        self.recv().await
    }
}
