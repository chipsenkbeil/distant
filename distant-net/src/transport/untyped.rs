use super::{Interest, Ready, Reconnectable, TypedTransport};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::io;

/// Interface representing a transport that uses [`serde`] to serialize and deserialize data
/// as it is sent and received
#[async_trait]
pub trait UntypedTransport: Reconnectable {
    /// Attempts to read some data as `T`, returning [`io::Error`] if unable to deserialize
    /// or some other error occurs. `Some(T)` is returned if successful. `None` is
    /// returned if no more data is available.
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to read data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    fn try_read<T>(&self) -> io::Result<Option<T>>
    where
        T: DeserializeOwned;

    /// Attempts to write some data `T` by serializing it into bytes, returning [`io::Error`] if
    /// unable to serialize or some other error occurs
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to write data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    fn try_write<T>(&self, value: &T) -> io::Result<()>
    where
        T: Serialize;

    /// Waits for the transport to be ready based on the given interest, returning the ready status
    async fn ready(&self, interest: Interest) -> io::Result<Ready>;

    /// Waits for the transport to be readable to follow up with `try_read`
    async fn readable(&self) -> io::Result<()> {
        let _ = self.ready(Interest::READABLE).await?;
        Ok(())
    }

    /// Waits for the transport to be writable to follow up with `try_write`
    async fn writeable(&self) -> io::Result<()> {
        let _ = self.ready(Interest::WRITABLE).await?;
        Ok(())
    }
}
