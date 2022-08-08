use crate::{
    FramedTransport, IntoSplit, PlainCodec, RawTransport, RawTransportRead, RawTransportWrite,
};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
};

mod read;
pub use read::*;

mod write;
pub use write::*;

/// Represents a [`RawTransport`] comprised of two inmemory channels
#[derive(Debug)]
pub struct InmemoryTransport {
    incoming: InmemoryTransportReadHalf,
    outgoing: InmemoryTransportWriteHalf,
}

impl InmemoryTransport {
    pub fn new(incoming: mpsc::Receiver<Vec<u8>>, outgoing: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            incoming: InmemoryTransportReadHalf::new(incoming),
            outgoing: InmemoryTransportWriteHalf::new(outgoing),
        }
    }

    /// Returns (incoming_tx, outgoing_rx, transport)
    pub fn make(buffer: usize) -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>, Self) {
        let (incoming_tx, incoming_rx) = mpsc::channel(buffer);
        let (outgoing_tx, outgoing_rx) = mpsc::channel(buffer);

        (
            incoming_tx,
            outgoing_rx,
            Self::new(incoming_rx, outgoing_tx),
        )
    }

    /// Returns pair of transports that are connected such that one sends to the other and
    /// vice versa
    pub fn pair(buffer: usize) -> (Self, Self) {
        let (tx, rx, transport) = Self::make(buffer);
        (transport, Self::new(rx, tx))
    }
}

impl RawTransport for InmemoryTransport {}
impl RawTransportRead for InmemoryTransport {}
impl RawTransportWrite for InmemoryTransport {}
impl IntoSplit for InmemoryTransport {
    type Read = InmemoryTransportReadHalf;
    type Write = InmemoryTransportWriteHalf;

    fn into_split(self) -> (Self::Write, Self::Read) {
        (self.outgoing, self.incoming)
    }
}

impl AsyncRead for InmemoryTransport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.incoming).poll_read(cx, buf)
    }
}

impl AsyncWrite for InmemoryTransport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.outgoing).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.outgoing).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.outgoing).poll_shutdown(cx)
    }
}

impl FramedTransport<InmemoryTransport, PlainCodec> {
    /// Produces a pair of inmemory transports that are connected to each other using
    /// a standard codec
    ///
    /// Sets the buffer for message passing for each underlying transport to the given buffer size
    pub fn pair(
        buffer: usize,
    ) -> (
        FramedTransport<InmemoryTransport, PlainCodec>,
        FramedTransport<InmemoryTransport, PlainCodec>,
    ) {
        let (a, b) = InmemoryTransport::pair(buffer);
        let a = FramedTransport::new(a, PlainCodec::new());
        let b = FramedTransport::new(b, PlainCodec::new());
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn make_should_return_sender_that_sends_data_to_transport() {
        let (tx, _, mut transport) = InmemoryTransport::make(3);

        tx.send(b"test msg 1".to_vec()).await.unwrap();
        tx.send(b"test msg 2".to_vec()).await.unwrap();
        tx.send(b"test msg 3".to_vec()).await.unwrap();

        // Should get data matching a singular message
        let mut buf = [0; 256];
        let len = transport.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"test msg 1");

        // Next call would get the second message
        let len = transport.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"test msg 2");

        // When the last of the senders is dropped, we should still get
        // the rest of the data that was sent first before getting
        // an indicator that there is no more data
        drop(tx);

        let len = transport.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"test msg 3");

        let len = transport.read(&mut buf).await.unwrap();
        assert_eq!(len, 0, "Unexpectedly got more data");
    }

    #[tokio::test]
    async fn make_should_return_receiver_that_receives_data_from_transport() {
        let (_, mut rx, mut transport) = InmemoryTransport::make(3);

        transport.write_all(b"test msg 1").await.unwrap();
        transport.write_all(b"test msg 2").await.unwrap();
        transport.write_all(b"test msg 3").await.unwrap();

        // Should get data matching a singular message
        assert_eq!(rx.recv().await, Some(b"test msg 1".to_vec()));

        // Next call would get the second message
        assert_eq!(rx.recv().await, Some(b"test msg 2".to_vec()));

        // When the transport is dropped, we should still get
        // the rest of the data that was sent first before getting
        // an indicator that there is no more data
        drop(transport);

        assert_eq!(rx.recv().await, Some(b"test msg 3".to_vec()));

        assert_eq!(rx.recv().await, None, "Unexpectedly got more data");
    }

    #[tokio::test]
    async fn into_split_should_provide_a_read_half_that_receives_from_sender() {
        let (tx, _, transport) = InmemoryTransport::make(3);
        let (_, mut read_half) = transport.into_split();

        tx.send(b"test msg 1".to_vec()).await.unwrap();
        tx.send(b"test msg 2".to_vec()).await.unwrap();
        tx.send(b"test msg 3".to_vec()).await.unwrap();

        // Should get data matching a singular message
        let mut buf = [0; 256];
        let len = read_half.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"test msg 1");

        // Next call would get the second message
        let len = read_half.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"test msg 2");

        // When the last of the senders is dropped, we should still get
        // the rest of the data that was sent first before getting
        // an indicator that there is no more data
        drop(tx);

        let len = read_half.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"test msg 3");

        let len = read_half.read(&mut buf).await.unwrap();
        assert_eq!(len, 0, "Unexpectedly got more data");
    }

    #[tokio::test]
    async fn into_split_should_provide_a_write_half_that_sends_to_receiver() {
        let (_, mut rx, transport) = InmemoryTransport::make(3);
        let (mut write_half, _) = transport.into_split();

        write_half.write_all(b"test msg 1").await.unwrap();
        write_half.write_all(b"test msg 2").await.unwrap();
        write_half.write_all(b"test msg 3").await.unwrap();

        // Should get data matching a singular message
        assert_eq!(rx.recv().await, Some(b"test msg 1".to_vec()));

        // Next call would get the second message
        assert_eq!(rx.recv().await, Some(b"test msg 2".to_vec()));

        // When the transport is dropped, we should still get
        // the rest of the data that was sent first before getting
        // an indicator that there is no more data
        drop(write_half);

        assert_eq!(rx.recv().await, Some(b"test msg 3".to_vec()));

        assert_eq!(rx.recv().await, None, "Unexpectedly got more data");
    }
}
