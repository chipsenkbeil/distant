use crate::{IntoSplit, TypedAsyncRead, TypedAsyncWrite};
use async_trait::async_trait;
use std::io;
use tokio::sync::mpsc;

mod read;
pub use read::*;

mod write;
pub use write::*;

/// Represents a transport of data across the network that uses [`mpsc::Sender`] and
/// [`mpsc::Receiver`] underneath.
#[derive(Debug)]
pub struct MpscTransport<T, U> {
    outbound: MpscTransportWriteHalf<T>,
    inbound: MpscTransportReadHalf<U>,
}

impl<T, U> MpscTransport<T, U> {
    pub fn new(outbound: mpsc::Sender<T>, inbound: mpsc::Receiver<U>) -> Self {
        Self {
            outbound: MpscTransportWriteHalf::new(outbound),
            inbound: MpscTransportReadHalf::new(inbound),
        }
    }
}

#[async_trait]
impl<T: Send, U: Send> TypedAsyncWrite<T> for MpscTransport<T, U> {
    async fn send(&mut self, data: T) -> io::Result<()> {
        self.outbound
            .send(data)
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
    }
}

#[async_trait]
impl<T: Send, U: Send> TypedAsyncRead<U> for MpscTransport<T, U> {
    async fn recv(&mut self) -> io::Result<Option<U>> {
        self.inbound.recv().await
    }
}

impl<T, U> IntoSplit for MpscTransport<T, U> {
    type Left = MpscTransportReadHalf<U>;
    type Right = MpscTransportWriteHalf<T>;

    fn into_split(self) -> (Self::Left, Self::Right) {
        (self.inbound, self.outbound)
    }
}
