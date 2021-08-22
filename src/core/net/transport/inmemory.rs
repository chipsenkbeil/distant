use super::DataStream;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{self, AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
};

/// Represents a data stream comprised of two inmemory channels
pub struct InmemoryStream {
    incoming: InmemoryStreamReadHalf,
    outgoing: InmemoryStreamWriteHalf,
}

impl InmemoryStream {
    pub fn new(incoming: mpsc::Receiver<Vec<u8>>, outgoing: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            incoming: InmemoryStreamReadHalf(incoming),
            outgoing: InmemoryStreamWriteHalf(outgoing),
        }
    }

    /// Returns (incoming_tx, outgoing_rx, stream)
    pub fn make(buffer: usize) -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>, Self) {
        let (incoming_tx, incoming_rx) = mpsc::channel(buffer);
        let (outgoing_tx, outgoing_rx) = mpsc::channel(buffer);

        (
            incoming_tx,
            outgoing_rx,
            Self::new(incoming_rx, outgoing_tx),
        )
    }

    /// Returns pair of streams that are connected such that one sends to the other and
    /// vice versa
    pub fn pair(buffer: usize) -> (Self, Self) {
        let (tx, rx, stream) = Self::make(buffer);
        (stream, Self::new(rx, tx))
    }
}

impl AsyncRead for InmemoryStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.incoming).poll_read(cx, buf)
    }
}

impl AsyncWrite for InmemoryStream {
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

/// Read portion of an inmemory channel
pub struct InmemoryStreamReadHalf(mpsc::Receiver<Vec<u8>>);

impl AsyncRead for InmemoryStreamReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.0.poll_recv(cx).map(|x| match x {
            Some(x) => {
                buf.put_slice(&x);
                Ok(())
            }
            None => Ok(()),
        })
    }
}

/// Write portion of an inmemory channel
pub struct InmemoryStreamWriteHalf(mpsc::Sender<Vec<u8>>);

impl AsyncWrite for InmemoryStreamWriteHalf {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.0.try_send(buf.to_vec()) {
            Ok(_) => Poll::Ready(Ok(buf.len())),
            Err(_) => Poll::Ready(Ok(0)),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

impl DataStream for InmemoryStream {
    type Read = InmemoryStreamReadHalf;
    type Write = InmemoryStreamWriteHalf;

    fn to_connection_tag(&self) -> String {
        String::from("test-stream")
    }

    fn into_split(self) -> (Self::Read, Self::Write) {
        (self.incoming, self.outgoing)
    }
}
