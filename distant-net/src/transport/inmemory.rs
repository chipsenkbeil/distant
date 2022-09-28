use super::{Interest, Ready, Reconnectable, Transport};
use async_trait::async_trait;
use std::{
    io,
    sync::{Mutex, MutexGuard},
};
use tokio::sync::mpsc::{
    self,
    error::{TryRecvError, TrySendError},
};

/// Represents a [`Transport`] comprised of two inmemory channels
#[derive(Debug)]
pub struct InmemoryTransport {
    tx: mpsc::Sender<Vec<u8>>,
    rx: Mutex<mpsc::Receiver<Vec<u8>>>,

    /// Internal storage used when we get more data from a `try_read` than can be returned
    buf: Mutex<Option<Vec<u8>>>,
}

impl InmemoryTransport {
    pub fn new(tx: mpsc::Sender<Vec<u8>>, rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            tx,
            rx: Mutex::new(rx),
            buf: Mutex::new(None),
        }
    }

    /// Returns (incoming_tx, outgoing_rx, transport)
    pub fn make(buffer: usize) -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>, Self) {
        let (incoming_tx, incoming_rx) = mpsc::channel(buffer);
        let (outgoing_tx, outgoing_rx) = mpsc::channel(buffer);

        (
            incoming_tx,
            outgoing_rx,
            Self::new(outgoing_tx, incoming_rx),
        )
    }

    /// Returns pair of transports that are connected such that one sends to the other and
    /// vice versa
    pub fn pair(buffer: usize) -> (Self, Self) {
        let (tx, rx, transport) = Self::make(buffer);
        (transport, Self::new(tx, rx))
    }

    /// Returns true if the read channel is closed, meaning it will no longer receive more data.
    /// This does not factor in data remaining in the internal buffer, meaning that this may return
    /// true while the transport still has data remaining in the internal buffer.
    ///
    /// NOTE: Because there is no `is_closed` on the receiver, we have to actually try to
    ///       read from the receiver to see if it is disconnected, adding any received data
    ///       to our internal buffer if it is not disconnected and has data available
    ///
    /// Track https://github.com/tokio-rs/tokio/issues/4638 for future `is_closed` on rx
    fn is_rx_closed(&self) -> bool {
        match self.rx.lock().unwrap().try_recv() {
            Ok(mut data) => {
                let mut buf_lock = self.buf.lock().unwrap();

                let data = match buf_lock.take() {
                    Some(mut existing) => {
                        existing.append(&mut data);
                        existing
                    }
                    None => data,
                };

                *buf_lock = Some(data);

                false
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => true,
        }
    }
}

#[async_trait]
impl Reconnectable for InmemoryTransport {
    /// Once the underlying channels have closed, there is no way for this transport to
    /// re-establish those channels; therefore, reconnecting will always fail with
    /// [`ErrorKind::Unsupported`]
    ///
    /// [`ErrorKind::Unsupported`]: io::ErrorKind::Unsupported
    async fn reconnect(&mut self) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

#[async_trait]
impl Transport for InmemoryTransport {
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        // Lock our internal storage to ensure that nothing else mutates it for the lifetime of
        // this call as we want to make sure that data is read and stored in order
        let mut buf_lock = self.buf.lock().unwrap();

        // Check if we have data in our internal buffer, and if so feed it into the outgoing buf
        if let Some(data) = buf_lock.take() {
            return Ok(copy_and_store(buf_lock, data, buf));
        }

        match self.rx.lock().unwrap().try_recv() {
            Ok(data) => Ok(copy_and_store(buf_lock, data, buf)),
            Err(TryRecvError::Empty) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Err(TryRecvError::Disconnected) => Ok(0),
        }
    }

    fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        match self.tx.try_send(buf.to_vec()) {
            Ok(()) => Ok(buf.len()),
            Err(TrySendError::Full(_)) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Err(TrySendError::Closed(_)) => Ok(0),
        }
    }

    async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        let mut status = Ready::EMPTY;

        if interest.is_readable() {
            // TODO: Replace `self.is_rx_closed()` with `self.rx.is_closed()` once the tokio issue
            //       is resolved that adds `is_closed` to the `mpsc::Receiver`
            //
            // See https://github.com/tokio-rs/tokio/issues/4638
            status |= if self.is_rx_closed() && self.buf.lock().unwrap().is_none() {
                Ready::READ_CLOSED
            } else {
                Ready::READABLE
            };
        }

        if interest.is_writable() {
            status |= if self.tx.is_closed() {
                Ready::WRITE_CLOSED
            } else {
                Ready::WRITABLE
            };
        }

        Ok(status)
    }
}

/// Copies `data` into `out`, storing any overflow from `data` into the storage pointed to by the
/// mutex `buf_lock`
fn copy_and_store(
    mut buf_lock: MutexGuard<Option<Vec<u8>>>,
    mut data: Vec<u8>,
    out: &mut [u8],
) -> usize {
    // NOTE: We can get data that is larger than the destination buf; so,
    //       we store as much as we can and queue up the rest in our temporary
    //       storage for future retrievals
    if data.len() > out.len() {
        let n = out.len();
        out.copy_from_slice(&data[..n]);
        *buf_lock = Some(data.split_off(n));
        n
    } else {
        let n = data.len();
        out[..n].copy_from_slice(&data);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    #[test]
    fn is_rx_closed_should_properly_reflect_if_internal_rx_channel_is_closed() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (read_tx, read_rx) = mpsc::channel(1);

        let transport = InmemoryTransport::new(write_tx, read_rx);

        // Not closed when the channel is empty
        assert!(!transport.is_rx_closed());

        read_tx.try_send(b"some bytes".to_vec()).unwrap();

        // Not closed when the channel has data (will queue up data)
        assert!(!transport.is_rx_closed());
        assert_eq!(
            transport.buf.lock().unwrap().as_deref().unwrap(),
            b"some bytes"
        );

        // Queue up one more set of bytes and then close the channel
        read_tx.try_send(b"more".to_vec()).unwrap();
        drop(read_tx);

        // Not closed when channel has closed but has something remaining in the queue
        assert!(!transport.is_rx_closed());
        assert_eq!(
            transport.buf.lock().unwrap().as_deref().unwrap(),
            b"some bytesmore"
        );

        // Closed once there is nothing left in the channel and it has closed
        assert!(transport.is_rx_closed());
        assert_eq!(
            transport.buf.lock().unwrap().as_deref().unwrap(),
            b"some bytesmore"
        );
    }

    #[test]
    fn try_read_should_succeed_if_able_to_read_entire_data_through_channel() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (read_tx, read_rx) = mpsc::channel(1);

        let transport = InmemoryTransport::new(write_tx, read_rx);

        // Queue up some data to be read
        read_tx.try_send(b"some bytes".to_vec()).unwrap();

        let mut buf = [0; 10];
        assert_eq!(transport.try_read(&mut buf).unwrap(), 10);
        assert_eq!(&buf[..10], b"some bytes");
    }

    #[test]
    fn try_read_should_succeed_if_reading_cached_data_from_previous_read() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (read_tx, read_rx) = mpsc::channel(1);

        let transport = InmemoryTransport::new(write_tx, read_rx);

        // Queue up some data to be read
        read_tx.try_send(b"some bytes".to_vec()).unwrap();

        let mut buf = [0; 5];
        assert_eq!(transport.try_read(&mut buf).unwrap(), 5);
        assert_eq!(&buf[..5], b"some ");

        // Queue up some new data to be read (previous data already consumed)
        read_tx.try_send(b"more".to_vec()).unwrap();

        let mut buf = [0; 2];
        assert_eq!(transport.try_read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], b"by");

        // Inmemory still separates buffered bytes from next channel recv()
        let mut buf = [0; 5];
        assert_eq!(transport.try_read(&mut buf).unwrap(), 3);
        assert_eq!(&buf[..3], b"tes");

        let mut buf = [0; 5];
        assert_eq!(transport.try_read(&mut buf).unwrap(), 4);
        assert_eq!(&buf[..4], b"more");
    }

    #[test]
    fn try_read_should_fail_with_would_block_if_channel_is_empty() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (_read_tx, read_rx) = mpsc::channel(1);

        let transport = InmemoryTransport::new(write_tx, read_rx);

        assert_eq!(
            transport.try_read(&mut [0; 5]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn try_read_should_succeed_with_zero_bytes_read_if_channel_closed() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (read_tx, read_rx) = mpsc::channel(1);

        // Drop to close the read channel
        drop(read_tx);

        let transport = InmemoryTransport::new(write_tx, read_rx);
        assert_eq!(transport.try_read(&mut [0; 5]).unwrap(), 0);
    }

    #[test]
    fn try_write_should_succeed_if_able_to_send_data_through_channel() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (_read_tx, read_rx) = mpsc::channel(1);

        let transport = InmemoryTransport::new(write_tx, read_rx);

        let value = b"some bytes";
        assert_eq!(transport.try_write(value).unwrap(), value.len());
    }

    #[test]
    fn try_write_should_fail_with_would_block_if_channel_capacity_has_been_reached() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (_read_tx, read_rx) = mpsc::channel(1);

        let transport = InmemoryTransport::new(write_tx, read_rx);

        // Fill up the channel
        transport
            .try_write(b"some bytes")
            .expect("Failed to fill channel");

        assert_eq!(
            transport.try_write(b"some bytes").unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn try_write_should_succeed_with_zero_bytes_written_if_channel_closed() {
        let (write_tx, write_rx) = mpsc::channel(1);
        let (_read_tx, read_rx) = mpsc::channel(1);

        // Drop to close the write channel
        drop(write_rx);

        let transport = InmemoryTransport::new(write_tx, read_rx);
        assert_eq!(transport.try_write(b"some bytes").unwrap(), 0);
    }

    #[test(tokio::test)]
    async fn reconnect_should_fail_as_unsupported() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (_read_tx, read_rx) = mpsc::channel(1);
        let mut transport = InmemoryTransport::new(write_tx, read_rx);

        assert_eq!(
            transport.reconnect().await.unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[test(tokio::test)]
    async fn ready_should_report_read_closed_if_channel_closed_and_internal_buf_empty() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (read_tx, read_rx) = mpsc::channel(1);

        // Drop to close the read channel
        drop(read_tx);

        let transport = InmemoryTransport::new(write_tx, read_rx);
        let ready = transport.ready(Interest::READABLE).await.unwrap();
        assert!(ready.is_readable());
        assert!(ready.is_read_closed());
    }

    #[test(tokio::test)]
    async fn ready_should_report_readable_if_channel_not_closed() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (_read_tx, read_rx) = mpsc::channel(1);

        let transport = InmemoryTransport::new(write_tx, read_rx);
        let ready = transport.ready(Interest::READABLE).await.unwrap();
        assert!(ready.is_readable());
        assert!(!ready.is_read_closed());
    }

    #[test(tokio::test)]
    async fn ready_should_report_readable_if_internal_buf_not_empty() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (read_tx, read_rx) = mpsc::channel(1);

        // Drop to close the read channel
        drop(read_tx);

        let transport = InmemoryTransport::new(write_tx, read_rx);

        // Assign some data to our buffer to ensure that we test this condition
        *transport.buf.lock().unwrap() = Some(vec![1]);

        let ready = transport.ready(Interest::READABLE).await.unwrap();
        assert!(ready.is_readable());
        assert!(!ready.is_read_closed());
    }

    #[test(tokio::test)]
    async fn ready_should_report_writable_if_channel_not_closed() {
        let (write_tx, _write_rx) = mpsc::channel(1);
        let (_read_tx, read_rx) = mpsc::channel(1);

        let transport = InmemoryTransport::new(write_tx, read_rx);
        let ready = transport.ready(Interest::WRITABLE).await.unwrap();
        assert!(ready.is_writable());
        assert!(!ready.is_write_closed());
    }

    #[test(tokio::test)]
    async fn ready_should_report_write_closed_if_channel_closed() {
        let (write_tx, write_rx) = mpsc::channel(1);
        let (_read_tx, read_rx) = mpsc::channel(1);

        // Drop to close the write channel
        drop(write_rx);

        let transport = InmemoryTransport::new(write_tx, read_rx);
        let ready = transport.ready(Interest::WRITABLE).await.unwrap();
        assert!(ready.is_writable());
        assert!(ready.is_write_closed());
    }

    #[test(tokio::test)]
    async fn make_should_return_sender_that_sends_data_to_transport() {
        let (tx, _, transport) = InmemoryTransport::make(3);

        tx.send(b"test msg 1".to_vec()).await.unwrap();
        tx.send(b"test msg 2".to_vec()).await.unwrap();
        tx.send(b"test msg 3".to_vec()).await.unwrap();

        // Should get data matching a singular message
        let mut buf = [0; 256];
        let len = transport.try_read(&mut buf).unwrap();
        assert_eq!(&buf[..len], b"test msg 1");

        // Next call would get the second message
        let len = transport.try_read(&mut buf).unwrap();
        assert_eq!(&buf[..len], b"test msg 2");

        // When the last of the senders is dropped, we should still get
        // the rest of the data that was sent first before getting
        // an indicator that there is no more data
        drop(tx);

        let len = transport.try_read(&mut buf).unwrap();
        assert_eq!(&buf[..len], b"test msg 3");

        let len = transport.try_read(&mut buf).unwrap();
        assert_eq!(len, 0, "Unexpectedly got more data");
    }

    #[test(tokio::test)]
    async fn make_should_return_receiver_that_receives_data_from_transport() {
        let (_, mut rx, transport) = InmemoryTransport::make(3);

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
}
