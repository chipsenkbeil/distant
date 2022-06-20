use crate::{IntoSplit, TypedAsyncRead, TypedAsyncWrite, TypedTransport};
use async_trait::async_trait;
use std::io;
use tokio::sync::mpsc;

mod read;
pub use read::*;

mod write;
pub use write::*;

/// Represents a [`TypedTransport`] of data across the network that uses [`mpsc::Sender`] and
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

    /// Creates a pair of connected transports using `buffer` as maximum
    /// channel capacity for each
    pub fn pair(buffer: usize) -> (MpscTransport<T, U>, MpscTransport<U, T>) {
        let (t_tx, t_rx) = mpsc::channel(buffer);
        let (u_tx, u_rx) = mpsc::channel(buffer);
        (
            MpscTransport::new(t_tx, u_rx),
            MpscTransport::new(u_tx, t_rx),
        )
    }
}

impl<T, U> TypedTransport<T, U> for MpscTransport<T, U>
where
    T: Send,
    U: Send,
{
    type ReadHalf = MpscTransportReadHalf<U>;
    type WriteHalf = MpscTransportWriteHalf<T>;
}

#[async_trait]
impl<T: Send, U: Send> TypedAsyncWrite<T> for MpscTransport<T, U> {
    async fn write(&mut self, data: T) -> io::Result<()> {
        self.outbound
            .write(data)
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
    }
}

#[async_trait]
impl<T: Send, U: Send> TypedAsyncRead<U> for MpscTransport<T, U> {
    async fn read(&mut self) -> io::Result<Option<U>> {
        self.inbound.read().await
    }
}

impl<T, U> IntoSplit for MpscTransport<T, U> {
    type Read = MpscTransportReadHalf<U>;
    type Write = MpscTransportWriteHalf<T>;

    fn into_split(self) -> (Self::Write, Self::Read) {
        (self.outbound, self.inbound)
    }
}
