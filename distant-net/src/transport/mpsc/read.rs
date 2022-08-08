use crate::TypedAsyncRead;
use async_trait::async_trait;
use std::io;
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct MpscTransportReadHalf<T> {
    rx: mpsc::Receiver<T>,
}

impl<T> MpscTransportReadHalf<T> {
    pub fn new(rx: mpsc::Receiver<T>) -> Self {
        Self { rx }
    }
}

#[async_trait]
impl<T: Send> TypedAsyncRead<T> for MpscTransportReadHalf<T> {
    async fn read(&mut self) -> io::Result<Option<T>> {
        Ok(self.rx.recv().await)
    }
}
