use super::{Interest, Ready, Reconnectable};
use async_trait::async_trait;
use std::io;

mod framed;
pub use framed::*;

mod inmemory;
pub use inmemory::*;

mod tcp;
pub use tcp::*;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::*;

/// Interface representing a transport of raw bytes into and out of the system
#[async_trait]
pub trait RawTransport: Reconnectable {
    /// Tries to read data from the transport into the provided buffer, returning how many bytes
    /// were read
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to read data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize>;

    /// Try to write a buffer to the transport, returning how many bytes were written
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to write data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    fn try_write(&self, buf: &[u8]) -> io::Result<usize>;

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
