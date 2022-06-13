use crate::TypedAsyncWrite;
use async_trait::async_trait;
use std::io;
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct MpscTransportWriteHalf<T> {
    tx: mpsc::Sender<T>,
}

impl<T> MpscTransportWriteHalf<T> {
    pub fn new(tx: mpsc::Sender<T>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl<T: Send> TypedAsyncWrite<T> for MpscTransportWriteHalf<T> {
    async fn send(&mut self, data: T) -> io::Result<()> {
        self.tx
            .send(data)
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x.to_string()))
    }
}
