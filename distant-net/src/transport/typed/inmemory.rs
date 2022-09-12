use super::{Interest, Ready, Reconnectable, TypedTransport};
use async_trait::async_trait;
use std::{io, sync::Mutex};
use tokio::sync::mpsc::{
    self,
    error::{TryRecvError, TrySendError},
};

/// Represents a [`TypedTransport`] of data across the network that uses tokio's mpsc [`Sender`]
/// and [`Receiver`] underneath.
///
/// [`Sender`]: mpsc::Sender
/// [`Receiver`]: mpsc::Receiver
#[derive(Debug)]
pub struct InmemoryTypedTransport<T, U> {
    tx: mpsc::Sender<T>,
    rx: Mutex<mpsc::Receiver<U>>,
}

impl<T, U> InmemoryTypedTransport<T, U> {
    pub fn new(tx: mpsc::Sender<T>, rx: mpsc::Receiver<U>) -> Self {
        Self {
            tx,
            rx: Mutex::new(rx),
        }
    }

    /// Creates a pair of connected transports using `buffer` as maximum
    /// channel capacity for each
    pub fn pair(buffer: usize) -> (InmemoryTypedTransport<T, U>, InmemoryTypedTransport<U, T>) {
        let (t_tx, t_rx) = mpsc::channel(buffer);
        let (u_tx, u_rx) = mpsc::channel(buffer);
        (
            InmemoryTypedTransport::new(t_tx, u_rx),
            InmemoryTypedTransport::new(u_tx, t_rx),
        )
    }
}

#[async_trait]
impl<T, U> Reconnectable for InmemoryTypedTransport<T, U>
where
    T: Send,
    U: Send,
{
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
impl<T, U> TypedTransport for InmemoryTypedTransport<T, U>
where
    T: Send,
    U: Send,
{
    type Input = U;
    type Output = T;

    fn try_read(&self) -> io::Result<Option<Self::Input>> {
        match self.rx.lock().unwrap().try_recv() {
            Ok(x) => Ok(Some(x)),
            Err(TryRecvError::Empty) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Err(TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn try_write(&self, value: Self::Output) -> io::Result<()> {
        match self.tx.try_send(value) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Err(TrySendError::Closed(_)) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
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
