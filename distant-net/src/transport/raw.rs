use super::{Interest, Ready, Reconnectable};
use async_trait::async_trait;
use std::io;

/* mod framed;
pub use framed::*; */

/* mod inmemory;
pub use inmemory::*; */

mod tcp;
pub use tcp::*;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

// #[cfg(windows)]
mod windows;

// #[cfg(windows)]
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

    /// Waits for the transport to be writeable to follow up with `try_write`
    async fn writeable(&self) -> io::Result<()> {
        let _ = self.ready(Interest::WRITABLE).await?;
        Ok(())
    }

    /// Reads exactly `n` bytes where `n` is the length of `buf` by continuing to call [`try_read`]
    /// until completed. Calls to [`readable`] are made to ensure the transport is ready. Returns
    /// the total bytes read.
    ///
    /// [`try_read`]: RawTransport::try_read
    /// [`readable`]: RawTransport::readable
    async fn read_exact(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut i = 0;

        while i < buf.len() {
            self.readable().await?;

            match self.try_read(&mut buf[i..]) {
                // If we get 0 bytes read, this usually means that the underlying reader
                // has closed, so we will return an EOF error to reflect that
                //
                // NOTE: `try_read` can also return 0 if the buf len is zero, but because we check
                //       that our index is < len, the situation where we call try_read with a buf
                //       of len 0 will never happen
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),

                Ok(n) => i += n,

                // Because we are using `try_read`, it can be possible for it to return
                // WouldBlock; so, if we encounter that then we just wait for next readable
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => continue,

                Err(x) => return Err(x),
            }
        }

        Ok(i)
    }

    /// Writes all of `buf` by continuing to call [`try_write`] until completed. Calls to
    /// [`writeable`] are made to ensure the transport is ready.
    ///
    /// [`try_write`]: RawTransport::try_write
    /// [`writable`]: RawTransport::writable
    async fn write_all(&self, buf: &[u8]) -> io::Result<()> {
        let mut i = 0;

        while i < buf.len() {
            self.writeable().await?;

            match self.try_write(&buf[i..]) {
                // If we get 0 bytes written, this usually means that the underlying writer
                // has closed, so we will return a broken pipe error to reflect that
                //
                // NOTE: `try_write` can also return 0 if the buf len is zero, but because we check
                //       that our index is < len, the situation where we call try_write with a buf
                //       of len 0 will never happen
                Ok(0) => return Err(io::Error::from(io::ErrorKind::BrokenPipe)),

                Ok(n) => i += n,

                // Because we are using `try_write`, it can be possible for it to return
                // WouldBlock; so, if we encounter that then we just wait for next writeable
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => continue,

                Err(x) => return Err(x),
            }
        }

        Ok(())
    }
}
