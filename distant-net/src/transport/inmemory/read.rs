use crate::RawTransportRead;
use futures::ready;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::mpsc,
};

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

impl RawTransportRead for InmemoryTransportReadHalf {}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InmemoryTransport, IntoSplit};
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn read_half_should_fail_if_buf_has_no_space_remaining() {
        let (_tx, _rx, transport) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = transport.into_split();

        let mut buf = [0u8; 0];
        match t_read.read(&mut buf).await {
            Err(x) if x.kind() == io::ErrorKind::Other => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn read_half_should_update_buf_with_all_overflow_from_last_read_if_it_all_fits() {
        let (tx, _rx, transport) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = transport.into_split();

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
        let (tx, _rx, transport) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = transport.into_split();

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
        let (tx, _rx, transport) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = transport.into_split();

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
        let (tx, _rx, transport) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = transport.into_split();

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
        let (_tx, _rx, transport) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = transport.into_split();

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
        let (tx, _rx, transport) = InmemoryTransport::make(1);
        let (mut t_read, _t_write) = transport.into_split();

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
}
