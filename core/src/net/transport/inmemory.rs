use super::{DataStream, SecretKey, Transport};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{self, AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
};

/// Represents a data stream comprised of two inmemory channels
#[derive(Debug)]
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
#[derive(Debug)]
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
#[derive(Debug)]
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
        String::from("inmemory-stream")
    }

    fn into_split(self) -> (Self::Read, Self::Write) {
        (self.incoming, self.outgoing)
    }
}

impl Transport<InmemoryStream> {
    /// Produces a pair of inmemory transports that are connected to each other with matching
    /// auth and encryption keys
    ///
    /// Sets the buffer for message passing for each underlying stream to the given buffer size
    pub fn pair(buffer: usize) -> (Transport<InmemoryStream>, Transport<InmemoryStream>) {
        let auth_key = Arc::new(SecretKey::default());
        let crypt_key = Arc::new(SecretKey::default());

        let (a, b) = InmemoryStream::pair(buffer);
        let a = Transport::new(a, Some(Arc::clone(&auth_key)), Arc::clone(&crypt_key));
        let b = Transport::new(b, Some(auth_key), crypt_key);
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn to_connection_tag_should_be_hardcoded_string() {
        let (_, _, stream) = InmemoryStream::make(1);
        assert_eq!(stream.to_connection_tag(), "inmemory-stream");
    }

    #[tokio::test]
    async fn make_should_return_sender_that_sends_data_to_stream() {
        let (tx, _, mut stream) = InmemoryStream::make(3);

        tx.send(b"test msg 1".to_vec()).await.unwrap();
        tx.send(b"test msg 2".to_vec()).await.unwrap();
        tx.send(b"test msg 3".to_vec()).await.unwrap();

        // Should get data matching a singular message
        let mut buf = [0; 256];
        let len = stream.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"test msg 1");

        // Next call would get the second message
        let len = stream.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"test msg 2");

        // When the last of the senders is dropped, we should still get
        // the rest of the data that was sent first before getting
        // an indicator that there is no more data
        drop(tx);

        let len = stream.read(&mut buf).await.unwrap();
        assert_eq!(&buf[..len], b"test msg 3");

        let len = stream.read(&mut buf).await.unwrap();
        assert_eq!(len, 0, "Unexpectedly got more data");
    }

    #[tokio::test]
    async fn make_should_return_receiver_that_receives_data_from_stream() {
        let (_, mut rx, mut stream) = InmemoryStream::make(3);

        stream.write_all(b"test msg 1").await.unwrap();
        stream.write_all(b"test msg 2").await.unwrap();
        stream.write_all(b"test msg 3").await.unwrap();

        // Should get data matching a singular message
        assert_eq!(rx.recv().await, Some(b"test msg 1".to_vec()));

        // Next call would get the second message
        assert_eq!(rx.recv().await, Some(b"test msg 2".to_vec()));

        // When the stream is dropped, we should still get
        // the rest of the data that was sent first before getting
        // an indicator that there is no more data
        drop(stream);

        assert_eq!(rx.recv().await, Some(b"test msg 3".to_vec()));

        assert_eq!(rx.recv().await, None, "Unexpectedly got more data");
    }

    #[tokio::test]
    async fn into_split_should_provide_a_read_half_that_receives_from_sender() {
        let (tx, _, stream) = InmemoryStream::make(3);
        let (mut read_half, _) = stream.into_split();

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
        let (_, mut rx, stream) = InmemoryStream::make(3);
        let (_, mut write_half) = stream.into_split();

        write_half.write_all(b"test msg 1").await.unwrap();
        write_half.write_all(b"test msg 2").await.unwrap();
        write_half.write_all(b"test msg 3").await.unwrap();

        // Should get data matching a singular message
        assert_eq!(rx.recv().await, Some(b"test msg 1".to_vec()));

        // Next call would get the second message
        assert_eq!(rx.recv().await, Some(b"test msg 2".to_vec()));

        // When the stream is dropped, we should still get
        // the rest of the data that was sent first before getting
        // an indicator that there is no more data
        drop(write_half);

        assert_eq!(rx.recv().await, Some(b"test msg 3".to_vec()));

        assert_eq!(rx.recv().await, None, "Unexpectedly got more data");
    }
}
