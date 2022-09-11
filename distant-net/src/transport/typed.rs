use super::{Interest, Ready, Reconnectable};
use async_trait::async_trait;
use std::io;

mod inmemory;
pub use inmemory::*;

/// Interface representing a transport of typed data into and out of the system
#[async_trait]
pub trait TypedTransport: Reconnectable {
    /// Type of input the transport can read
    type Input;

    /// Type of output the transport can write
    type Output;

    /// Tries to read a value from the transport, returning `Ok(Some(Self::Input))` upon
    /// acquiring new input, or `Ok(None)` if the channel has closed.
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to read data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    fn try_read(&self) -> io::Result<Option<Self::Input>>;

    /// Try to write a value to the transport, returning `Ok(())` upon successfully writing all of
    /// the data
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to write data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    fn try_write(&self, value: Self::Output) -> io::Result<()>;

    /// Waits for the transport to be ready based on the given interest, returning the ready status
    async fn ready(&self, interest: Interest) -> io::Result<Ready>;

    /// Waits for the transport to be readable to follow up with `try_read`
    async fn readable(&self) -> io::Result<()> {
        let _ = self.ready(Interest::READABLE).await?;
        Ok(())
    }

    /// Waits for the transport to be readable to follow up with `try_write`
    async fn writeable(&self) -> io::Result<()> {
        let _ = self.ready(Interest::WRITABLE).await?;
        Ok(())
    }
}
