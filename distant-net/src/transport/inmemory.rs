use crate::{FramedTransport, PlainCodec, Transport};
use futures::ready;
use std::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{self, AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
};

/// Represents a data stream comprised of two inmemory channels
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

/// Read portion of an inmemory channel
#[derive(Debug)]
pub struct InmemoryTransportReadHalf {
    rx: mpsc::Receiver<Vec<u8>>,
    overflow: Vec<u8>,
}

impl InmemoryTransportReadHalf {
    pub fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            overflow: Vec::new(),
        }
    }
}

impl AsyncRead for InmemoryTransportReadHalf {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // If we cannot fit any more into the buffer at the moment, we wait
        if buf.remaining() == 0 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::Other,
                "Cannot poll as buf.remaining() == 0",
            )));
        }

        // If we have overflow from the last poll, put that in the buffer
        if !self.overflow.is_empty() {
            if self.overflow.len() > buf.remaining() {
                let extra = self.overflow.split_off(buf.remaining());
                buf.put_slice(&self.overflow);
                self.overflow = extra;
            } else {
                buf.put_slice(&self.overflow);
                self.overflow.clear();
            }

            return Poll::Ready(Ok(()));
        }

        // Otherwise, we poll for the next batch to read in
        match ready!(self.rx.poll_recv(cx)) {
            Some(mut x) => {
                if x.len() > buf.remaining() {
                    self.overflow = x.split_off(buf.remaining());
                }
                buf.put_slice(&x);
                Poll::Ready(Ok(()))
            }
            None => Poll::Ready(Ok(())),
        }
    }
}

/// Write portion of an inmemory channel
pub struct InmemoryTransportWriteHalf {
    tx: Option<mpsc::Sender<Vec<u8>>>,
    task: Option<Pin<Box<dyn Future<Output = io::Result<usize>> + Send + Sync + 'static>>>,
}

impl InmemoryTransportWriteHalf {
    pub fn new(tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            tx: Some(tx),
            task: None,
        }
    }
}

impl fmt::Debug for InmemoryTransportWriteHalf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InmemoryTransportWrite")
            .field("tx", &self.tx)
            .field(
                "task",
                &if self.tx.is_some() {
                    "Some(...)"
                } else {
                    "None"
                },
            )
            .finish()
    }
}

impl AsyncWrite for InmemoryTransportWriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            match self.task.as_mut() {
                Some(task) => {
                    let res = ready!(task.as_mut().poll(cx));
                    self.task.take();
                    return Poll::Ready(res);
                }
                None => match self.tx.as_mut() {
                    Some(tx) => {
                        let n = buf.len();
                        let tx_2 = tx.clone();
                        let data = buf.to_vec();
                        let task =
                            Box::pin(async move { tx_2.send(data).await.map(|_| n).or(Ok(0)) });
                        self.task.replace(task);
                    }
                    None => return Poll::Ready(Ok(0)),
                },
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.tx.take();
        self.task.take();
        Poll::Ready(Ok(()))
    }
}

impl Transport for InmemoryTransport {
    type ReadHalf = InmemoryTransportReadHalf;
    type WriteHalf = InmemoryTransportWriteHalf;

    fn into_split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        (self.incoming, self.outgoing)
    }
}

impl FramedTransport<InmemoryTransport, PlainCodec> {
    /// Produces a pair of inmemory transports that are connected to each other using
    /// a standard codec
    ///
    /// Sets the buffer for message passing for each underlying stream to the given buffer size
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
    async fn make_should_return_sender_that_sends_data_to_stream() {
        let (tx, _, mut stream) = InmemoryTransport::make(3);

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
        let (_, mut rx, mut stream) = InmemoryTransport::make(3);

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
        let (tx, _, stream) = InmemoryTransport::make(3);
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
        let (_, mut rx, stream) = InmemoryTransport::make(3);
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

    #[tokio::test]
    async fn read_half_should_fail_if_buf_has_no_space_remaining() {
        let (_tx, _rx, stream) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = stream.into_split();

        let mut buf = [0u8; 0];
        match t_read.read(&mut buf).await {
            Err(x) if x.kind() == io::ErrorKind::Other => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn read_half_should_update_buf_with_all_overflow_from_last_read_if_it_all_fits() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = stream.into_split();

        tx.send(vec![1, 2, 3]).await.expect("Failed to send");

        let mut buf = [0u8; 2];

        // First, read part of the data (first two bytes)
        match t_read.read(&mut buf).await {
            Ok(n) if n == 2 => assert_eq!(&buf[..n], &[1, 2]),
            x => panic!("Unexpected result: {:?}", x),
        }

        // Second, we send more data because the last message was placed in overflow
        tx.send(vec![4, 5, 6]).await.expect("Failed to send");

        // Third, read remainder of the overflow from first message (third byte)
        match t_read.read(&mut buf).await {
            Ok(n) if n == 1 => assert_eq!(&buf[..n], &[3]),
            x => panic!("Unexpected result: {:?}", x),
        }

        // Fourth, verify that we start to receive the next overflow
        match t_read.read(&mut buf).await {
            Ok(n) if n == 2 => assert_eq!(&buf[..n], &[4, 5]),
            x => panic!("Unexpected result: {:?}", x),
        }

        // Fifth, verify that we get the last bit of overflow
        match t_read.read(&mut buf).await {
            Ok(n) if n == 1 => assert_eq!(&buf[..n], &[6]),
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn read_half_should_update_buf_with_some_of_overflow_that_can_fit() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = stream.into_split();

        tx.send(vec![1, 2, 3, 4, 5]).await.expect("Failed to send");

        let mut buf = [0u8; 2];

        // First, read part of the data (first two bytes)
        match t_read.read(&mut buf).await {
            Ok(n) if n == 2 => assert_eq!(&buf[..n], &[1, 2]),
            x => panic!("Unexpected result: {:?}", x),
        }

        // Second, we send more data because the last message was placed in overflow
        tx.send(vec![6]).await.expect("Failed to send");

        // Third, read next chunk of the overflow from first message (next two byte)
        match t_read.read(&mut buf).await {
            Ok(n) if n == 2 => assert_eq!(&buf[..n], &[3, 4]),
            x => panic!("Unexpected result: {:?}", x),
        }

        // Fourth, read last chunk of the overflow from first message (fifth byte)
        match t_read.read(&mut buf).await {
            Ok(n) if n == 1 => assert_eq!(&buf[..n], &[5]),
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn read_half_should_update_buf_with_all_of_inner_channel_when_it_fits() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = stream.into_split();

        let mut buf = [0u8; 5];

        tx.send(vec![1, 2, 3, 4, 5]).await.expect("Failed to send");

        // First, read all of data that fits exactly
        match t_read.read(&mut buf).await {
            Ok(n) if n == 5 => assert_eq!(&buf[..n], &[1, 2, 3, 4, 5]),
            x => panic!("Unexpected result: {:?}", x),
        }

        tx.send(vec![6, 7, 8]).await.expect("Failed to send");

        // Second, read data that fits within buf
        match t_read.read(&mut buf).await {
            Ok(n) if n == 3 => assert_eq!(&buf[..n], &[6, 7, 8]),
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn read_half_should_update_buf_with_some_of_inner_channel_that_can_fit_and_add_rest_to_overflow(
    ) {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = stream.into_split();

        let mut buf = [0u8; 1];

        tx.send(vec![1, 2, 3, 4, 5]).await.expect("Failed to send");

        // Attempt a read that places more in overflow
        match t_read.read(&mut buf).await {
            Ok(n) if n == 1 => assert_eq!(&buf[..n], &[1]),
            x => panic!("Unexpected result: {:?}", x),
        }

        // Verify overflow contains the rest
        assert_eq!(&t_read.overflow, &[2, 3, 4, 5]);

        // Queue up extra data that will not be read until overflow is finished
        tx.send(vec![6, 7, 8]).await.expect("Failed to send");

        // Read next data point
        match t_read.read(&mut buf).await {
            Ok(n) if n == 1 => assert_eq!(&buf[..n], &[2]),
            x => panic!("Unexpected result: {:?}", x),
        }

        // Verify overflow contains the rest without having added extra data
        assert_eq!(&t_read.overflow, &[3, 4, 5]);
    }

    #[tokio::test]
    async fn read_half_should_yield_pending_if_no_data_available_on_inner_channel() {
        let (_tx, _rx, stream) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = stream.into_split();

        let mut buf = [0u8; 1];

        // Attempt a read that should yield ok with no change, which is what should
        // happen when nothing is read into buf
        let f = t_read.read(&mut buf);
        tokio::pin!(f);
        match futures::poll!(f) {
            Poll::Pending => {}
            x => panic!("Unexpected poll result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn read_half_should_not_update_buf_if_inner_channel_closed() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = stream.into_split();

        let mut buf = [0u8; 1];

        // Drop the channel that would be sending data to the transport
        drop(tx);

        // Attempt a read that should yield ok with no change, which is what should
        // happen when nothing is read into buf
        match t_read.read(&mut buf).await {
            Ok(n) if n == 0 => assert_eq!(&buf, &[0]),
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn write_half_should_return_buf_len_if_can_send_immediately() {
        let (_tx, mut rx, stream) = InmemoryTransport::make(1);
        let (_t_read, mut t_write) = stream.into_split();

        // Write that is not waiting should always succeed with full contents
        let n = t_write.write(&[1, 2, 3]).await.expect("Failed to write");
        assert_eq!(n, 3, "Unexpected byte count returned");

        // Verify we actually had the data sent
        let data = rx.try_recv().expect("Failed to recv data");
        assert_eq!(data, &[1, 2, 3]);
    }

    #[tokio::test]
    async fn write_half_should_return_support_eventually_sending_by_retrying_when_not_ready() {
        let (_tx, mut rx, stream) = InmemoryTransport::make(1);
        let (_t_read, mut t_write) = stream.into_split();

        // Queue a write already so that we block on the next one
        let _ = t_write.write(&[1, 2, 3]).await.expect("Failed to write");

        // Verify that the next write is pending
        let f = t_write.write(&[4, 5]);
        tokio::pin!(f);
        match futures::poll!(&mut f) {
            Poll::Pending => {}
            x => panic!("Unexpected poll result: {:?}", x),
        }

        // Consume first batch of data so future of second can continue
        let data = rx.try_recv().expect("Failed to recv data");
        assert_eq!(data, &[1, 2, 3]);

        // Verify that poll now returns success
        match futures::poll!(f) {
            Poll::Ready(Ok(n)) if n == 2 => {}
            x => panic!("Unexpected poll result: {:?}", x),
        }

        // Consume second batch of data
        let data = rx.try_recv().expect("Failed to recv data");
        assert_eq!(data, &[4, 5]);
    }

    #[tokio::test]
    async fn write_half_should_zero_if_inner_channel_closed() {
        let (_tx, rx, stream) = InmemoryTransport::make(1);
        let (_t_read, mut t_write) = stream.into_split();

        // Drop receiving end that transport would talk to
        drop(rx);

        // Channel is dropped, so return 0 to indicate no bytes sent
        let n = t_write.write(&[1, 2, 3]).await.expect("Failed to write");
        assert_eq!(n, 0, "Unexpected byte count returned");
    }
}
