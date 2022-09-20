use async_trait::async_trait;
use std::io;

// mod router;

mod framed;
pub use framed::*;

mod inmemory;
pub use inmemory::*;

mod tcp;
pub use tcp::*;

mod stateful;
pub use stateful::*;

#[cfg(test)]
mod test;

#[cfg(test)]
pub use test::*;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::*;

pub use tokio::io::{Interest, Ready};

/// Interface representing a connection that is reconnectable
#[async_trait]
pub trait Reconnectable {
    /// Attempts to reconnect an already-established connection
    async fn reconnect(&mut self) -> io::Result<()>;
}

/// Interface representing a transport of raw bytes into and out of the system
#[async_trait]
pub trait Transport: Reconnectable {
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
        self.ready(Interest::READABLE).await?;
        Ok(())
    }

    /// Waits for the transport to be writeable to follow up with `try_write`
    async fn writeable(&self) -> io::Result<()> {
        self.ready(Interest::WRITABLE).await?;
        Ok(())
    }

    /// Reads exactly `n` bytes where `n` is the length of `buf` by continuing to call [`try_read`]
    /// until completed. Calls to [`readable`] are made to ensure the transport is ready. Returns
    /// the total bytes read.
    ///
    /// [`try_read`]: Transport::try_read
    /// [`readable`]: Transport::readable
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
    /// [`try_write`]: Transport::try_write
    /// [`writable`]: Transport::writable
    async fn write_all(&self, buf: &[u8]) -> io::Result<()> {
        let mut i = 0;

        while i < buf.len() {
            self.writeable().await?;

            match self.try_write(&buf[i..]) {
                // If we get 0 bytes written, this usually means that the underlying writer
                // has closed, so we will return a write zero error to reflect that
                //
                // NOTE: `try_write` can also return 0 if the buf len is zero, but because we check
                //       that our index is < len, the situation where we call try_write with a buf
                //       of len 0 will never happen
                Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),

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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_exact_should_fail_if_try_read_encounters_error_other_than_would_block() {
        let transport = TestTransport {
            f_try_read: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = [0; 1];
        assert_eq!(
            transport.read_exact(&mut buf).await.unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
    }

    #[tokio::test]
    async fn read_exact_should_fail_if_try_read_returns_0_before_necessary_bytes_read() {
        let transport = TestTransport {
            f_try_read: Box::new(|_| Ok(0)),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = [0; 1];
        assert_eq!(
            transport.read_exact(&mut buf).await.unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[tokio::test]
    async fn read_exact_should_continue_to_call_try_read_until_buffer_is_filled() {
        let transport = TestTransport {
            f_try_read: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    buf[0] = b'a' + CNT;
                    CNT += 1;
                }
                Ok(1)
            }),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = [0; 3];
        assert_eq!(transport.read_exact(&mut buf).await.unwrap(), 3);
        assert_eq!(&buf, b"abc");
    }

    #[tokio::test]
    async fn read_exact_should_continue_to_call_try_read_while_it_returns_would_block() {
        // Configure `try_read` to alternate between reading a byte and WouldBlock
        let transport = TestTransport {
            f_try_read: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    buf[0] = b'a' + CNT;
                    CNT += 1;
                    if CNT % 2 == 1 {
                        Ok(1)
                    } else {
                        Err(io::Error::from(io::ErrorKind::WouldBlock))
                    }
                }
            }),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = [0; 3];
        assert_eq!(transport.read_exact(&mut buf).await.unwrap(), 3);
        assert_eq!(&buf, b"ace");
    }

    #[tokio::test]
    async fn read_exact_should_return_0_if_given_a_buffer_of_0_len() {
        let transport = TestTransport {
            f_try_read: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = [0; 0];
        assert_eq!(transport.read_exact(&mut buf).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn write_all_should_fail_if_try_write_encounters_error_other_than_would_block() {
        let transport = TestTransport {
            f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
            f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
            ..Default::default()
        };

        assert_eq!(
            transport.write_all(b"abc").await.unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
    }

    #[tokio::test]
    async fn write_all_should_fail_if_try_write_returns_0_before_all_bytes_written() {
        let transport = TestTransport {
            f_try_write: Box::new(|_| Ok(0)),
            f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
            ..Default::default()
        };

        assert_eq!(
            transport.write_all(b"abc").await.unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
    }

    #[tokio::test]
    async fn write_all_should_continue_to_call_try_write_until_all_bytes_written() {
        // Configure `try_write` to alternate between writing a byte and WouldBlock
        let transport = TestTransport {
            f_try_write: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    assert_eq!(buf[0], b'a' + CNT);
                    CNT += 1;
                    Ok(1)
                }
            }),
            f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
            ..Default::default()
        };

        transport.write_all(b"abc").await.unwrap();
    }

    #[tokio::test]
    async fn write_all_should_continue_to_call_try_write_while_it_returns_would_block() {
        // Configure `try_write` to alternate between writing a byte and WouldBlock
        let transport = TestTransport {
            f_try_write: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    if CNT % 2 == 0 {
                        assert_eq!(buf[0], b'a' + CNT);
                        CNT += 1;
                        Ok(1)
                    } else {
                        CNT += 1;
                        Err(io::Error::from(io::ErrorKind::WouldBlock))
                    }
                }
            }),
            f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
            ..Default::default()
        };

        transport.write_all(b"ace").await.unwrap();
    }

    #[tokio::test]
    async fn write_all_should_return_immediately_if_given_buffer_of_0_len() {
        let transport = TestTransport {
            f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
            f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
            ..Default::default()
        };

        // No error takes place as we never call try_write
        let buf = [0; 0];
        transport.write_all(&buf).await.unwrap();
    }
}
