use super::{Interest, RawTransport, Ready, Reconnectable};
use async_trait::async_trait;
use std::{
    io,
    sync::{Mutex, MutexGuard},
};
use tokio::sync::mpsc::{
    self,
    error::{TryRecvError, TrySendError},
};

/// Represents a [`RawTransport`] comprised of two inmemory channels
#[derive(Debug)]
pub struct InmemoryTransport {
    tx: mpsc::Sender<Vec<u8>>,
    rx: mpsc::Receiver<Vec<u8>>,

    /// Internal storage used when we get more data from a `try_read` than can be returned
    buf: Mutex<Option<Vec<u8>>>,
}

impl InmemoryTransport {
    pub fn new(tx: mpsc::Sender<Vec<u8>>, rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            tx,
            rx,
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
        match self.rx.try_recv() {
            Ok(mut data) => {
                let buf_lock = self.buf.lock().unwrap();

                let data = match buf_lock.take() {
                    Some(existing) => {
                        existing.append(&mut data);
                        existing
                    }
                    None => data,
                };

                *buf_lock = Some(data);

                true
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
impl RawTransport for InmemoryTransport {
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        // Lock our internal storage to ensure that nothing else mutates it for the lifetime of
        // this call as we want to make sure that data is read and stored in order
        let buf_lock = self.buf.lock().unwrap();

        // Check if we have data in our internal buffer, and if so feed it into the outgoing buf
        if let Some(data) = buf_lock.take() {
            return Ok(copy_and_store(buf_lock, data, buf));
        }

        match self.rx.try_recv() {
            Ok(data) => Ok(copy_and_store(buf_lock, data, buf)),
            Err(TryRecvError::Empty) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Err(TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        match self.tx.try_send(buf.to_vec()) {
            Ok(()) => Ok(buf.len()),
            Err(TrySendError::Full(_)) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Err(TryRecvError::Closed(_)) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
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

        if interest.is_writeable() {
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
fn copy_and_store(buf_lock: MutexGuard<Option<Vec<u8>>>, data: Vec<u8>, out: &mut [u8]) -> usize {
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
}
