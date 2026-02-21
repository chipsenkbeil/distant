use std::time::Duration;
use std::{fmt, io};

use async_trait::async_trait;

mod framed;
pub use framed::*;

mod inmemory;
pub use inmemory::*;

mod tcp;
pub use tcp::*;

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

pub use tokio::io::{Interest, Ready};
#[cfg(windows)]
pub use windows::*;

/// Duration to wait after WouldBlock received during looping operations like `read_exact`.
const SLEEP_DURATION: Duration = Duration::from_millis(1);

/// Interface representing a connection that is reconnectable.
#[async_trait]
pub trait Reconnectable {
    /// Attempts to reconnect an already-established connection.
    async fn reconnect(&mut self) -> io::Result<()>;
}

/// Interface representing a transport of raw bytes into and out of the system.
#[async_trait]
pub trait Transport: Reconnectable + fmt::Debug + Send + Sync {
    /// Tries to read data from the transport into the provided buffer, returning how many bytes
    /// were read.
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to read data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize>;

    /// Try to write a buffer to the transport, returning how many bytes were written.
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to write data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    fn try_write(&self, buf: &[u8]) -> io::Result<usize>;

    /// Waits for the transport to be ready based on the given interest, returning the ready
    /// status.
    async fn ready(&self, interest: Interest) -> io::Result<Ready>;
}

#[async_trait]
impl Transport for Box<dyn Transport> {
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        Transport::try_read(AsRef::as_ref(self), buf)
    }

    fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        Transport::try_write(AsRef::as_ref(self), buf)
    }

    async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        Transport::ready(AsRef::as_ref(self), interest).await
    }
}

#[async_trait]
impl Reconnectable for Box<dyn Transport> {
    async fn reconnect(&mut self) -> io::Result<()> {
        Reconnectable::reconnect(AsMut::as_mut(self)).await
    }
}

#[async_trait]
pub trait TransportExt {
    /// Waits for the transport to be readable to follow up with `try_read`.
    async fn readable(&self) -> io::Result<()>;

    /// Waits for the transport to be writeable to follow up with `try_write`.
    async fn writeable(&self) -> io::Result<()>;

    /// Waits for the transport to be either readable or writeable.
    async fn readable_or_writeable(&self) -> io::Result<()>;

    /// Reads exactly `n` bytes where `n` is the length of `buf` by continuing to call [`try_read`]
    /// until completed. Calls to [`readable`] are made to ensure the transport is ready. Returns
    /// the total bytes read.
    ///
    /// [`try_read`]: Transport::try_read
    /// [`readable`]: Transport::readable
    async fn read_exact(&self, buf: &mut [u8]) -> io::Result<usize>;

    /// Reads all bytes until EOF in this source, placing them into `buf`.
    ///
    /// All bytes read from this source will be appended to the specified buffer `buf`. This
    /// function will continuously call [`try_read`] to append more data to `buf` until
    /// [`try_read`] returns either [`Ok(0)`] or an error that is neither [`Interrupted`] or
    /// [`WouldBlock`].
    ///
    /// If successful, this function will return the total number of bytes read.
    ///
    /// ### Errors
    ///
    /// If this function encounters an error of the kind [`Interrupted`] or [`WouldBlock`], then
    /// the error is ignored and the operation will continue.
    ///
    /// If any other read error is encountered then this function immediately returns. Any bytes
    /// which have already been read will be appended to `buf`.
    ///
    /// [`Ok(0)`]: Ok
    /// [`try_read`]: Transport::try_read
    /// [`readable`]: Transport::readable
    async fn read_to_end(&self, buf: &mut Vec<u8>) -> io::Result<usize>;

    /// Reads all bytes until EOF in this source, placing them into `buf`.
    ///
    /// If successful, this function will return the total number of bytes read.
    ///
    /// ### Errors
    ///
    /// If the data in this stream is *not* valid UTF-8 then an error is returned and `buf` is
    /// unchanged.
    ///
    /// See [`read_to_end`] for other error semantics.
    ///
    /// [`Ok(0)`]: Ok
    /// [`try_read`]: Transport::try_read
    /// [`readable`]: Transport::readable
    /// [`read_to_end`]: TransportExt::read_to_end
    async fn read_to_string(&self, buf: &mut String) -> io::Result<usize>;

    /// Writes all of `buf` by continuing to call [`try_write`] until completed. Calls to
    /// [`writeable`] are made to ensure the transport is ready.
    ///
    /// [`try_write`]: Transport::try_write
    /// [`writable`]: Transport::writable
    async fn write_all(&self, buf: &[u8]) -> io::Result<()>;
}

#[async_trait]
impl<T: Transport> TransportExt for T {
    async fn readable(&self) -> io::Result<()> {
        self.ready(Interest::READABLE).await?;
        Ok(())
    }

    async fn writeable(&self) -> io::Result<()> {
        self.ready(Interest::WRITABLE).await?;
        Ok(())
    }

    async fn readable_or_writeable(&self) -> io::Result<()> {
        self.ready(Interest::READABLE | Interest::WRITABLE).await?;
        Ok(())
    }

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
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                    // NOTE: We sleep for a little bit before trying again to avoid pegging CPU
                    tokio::time::sleep(SLEEP_DURATION).await
                }

                Err(x) => return Err(x),
            }
        }

        Ok(i)
    }

    async fn read_to_end(&self, buf: &mut Vec<u8>) -> io::Result<usize> {
        let mut i = 0;
        let mut tmp = [0u8; 1024];

        loop {
            self.readable().await?;

            match self.try_read(&mut tmp) {
                Ok(0) => return Ok(i),
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    i += n;
                }
                Err(x)
                    if x.kind() == io::ErrorKind::WouldBlock
                        || x.kind() == io::ErrorKind::Interrupted =>
                {
                    // NOTE: We sleep for a little bit before trying again to avoid pegging CPU
                    tokio::time::sleep(SLEEP_DURATION).await
                }

                Err(x) => return Err(x),
            }
        }
    }

    async fn read_to_string(&self, buf: &mut String) -> io::Result<usize> {
        let mut tmp = Vec::new();
        let n = self.read_to_end(&mut tmp).await?;
        buf.push_str(
            &String::from_utf8(tmp).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
        );
        Ok(n)
    }

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
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                    // NOTE: We sleep for a little bit before trying again to avoid pegging CPU
                    tokio::time::sleep(SLEEP_DURATION).await
                }

                Err(x) => return Err(x),
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test(tokio::test)]
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

    #[test(tokio::test)]
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

    #[test(tokio::test)]
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

    #[test(tokio::test)]
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

    #[test(tokio::test)]
    async fn read_exact_should_return_0_if_given_a_buffer_of_0_len() {
        let transport = TestTransport {
            f_try_read: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = [0; 0];
        assert_eq!(transport.read_exact(&mut buf).await.unwrap(), 0);
    }

    #[test(tokio::test)]
    async fn read_to_end_should_fail_if_try_read_encounters_error_other_than_would_block_and_interrupt(
    ) {
        let transport = TestTransport {
            f_try_read: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        assert_eq!(
            transport
                .read_to_end(&mut Vec::new())
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotConnected
        );
    }

    #[test(tokio::test)]
    async fn read_to_end_should_read_until_0_bytes_returned_from_try_read() {
        let transport = TestTransport {
            f_try_read: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    if CNT == 0 {
                        buf[..5].copy_from_slice(b"hello");
                        CNT += 1;
                        Ok(5)
                    } else {
                        Ok(0)
                    }
                }
            }),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = Vec::new();
        assert_eq!(transport.read_to_end(&mut buf).await.unwrap(), 5);
        assert_eq!(buf, b"hello");
    }

    #[test(tokio::test)]
    async fn read_to_end_should_continue_reading_when_interrupt_or_would_block_encountered() {
        let transport = TestTransport {
            f_try_read: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    CNT += 1;
                    if CNT == 1 {
                        buf[..6].copy_from_slice(b"hello ");
                        Ok(6)
                    } else if CNT == 2 {
                        Err(io::Error::from(io::ErrorKind::WouldBlock))
                    } else if CNT == 3 {
                        buf[..5].copy_from_slice(b"world");
                        Ok(5)
                    } else if CNT == 4 {
                        Err(io::Error::from(io::ErrorKind::Interrupted))
                    } else if CNT == 5 {
                        buf[..6].copy_from_slice(b", test");
                        Ok(6)
                    } else {
                        Ok(0)
                    }
                }
            }),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = Vec::new();
        assert_eq!(transport.read_to_end(&mut buf).await.unwrap(), 17);
        assert_eq!(buf, b"hello world, test");
    }

    #[test(tokio::test)]
    async fn read_to_string_should_fail_if_try_read_encounters_error_other_than_would_block_and_interrupt(
    ) {
        let transport = TestTransport {
            f_try_read: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        assert_eq!(
            transport
                .read_to_string(&mut String::new())
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotConnected
        );
    }

    #[test(tokio::test)]
    async fn read_to_string_should_fail_if_non_utf8_characters_read() {
        let transport = TestTransport {
            f_try_read: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    if CNT == 0 {
                        buf[0] = 0;
                        buf[1] = 159;
                        buf[2] = 146;
                        buf[3] = 150;
                        CNT += 1;
                        Ok(4)
                    } else {
                        Ok(0)
                    }
                }
            }),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = String::new();
        assert_eq!(
            transport.read_to_string(&mut buf).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test(tokio::test)]
    async fn read_to_string_should_read_until_0_bytes_returned_from_try_read() {
        let transport = TestTransport {
            f_try_read: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    if CNT == 0 {
                        buf[..5].copy_from_slice(b"hello");
                        CNT += 1;
                        Ok(5)
                    } else {
                        Ok(0)
                    }
                }
            }),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = String::new();
        assert_eq!(transport.read_to_string(&mut buf).await.unwrap(), 5);
        assert_eq!(buf, "hello");
    }

    #[test(tokio::test)]
    async fn read_to_string_should_continue_reading_when_interrupt_or_would_block_encountered() {
        let transport = TestTransport {
            f_try_read: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    CNT += 1;
                    if CNT == 1 {
                        buf[..6].copy_from_slice(b"hello ");
                        Ok(6)
                    } else if CNT == 2 {
                        Err(io::Error::from(io::ErrorKind::WouldBlock))
                    } else if CNT == 3 {
                        buf[..5].copy_from_slice(b"world");
                        Ok(5)
                    } else if CNT == 4 {
                        Err(io::Error::from(io::ErrorKind::Interrupted))
                    } else if CNT == 5 {
                        buf[..6].copy_from_slice(b", test");
                        Ok(6)
                    } else {
                        Ok(0)
                    }
                }
            }),
            f_ready: Box::new(|_| Ok(Ready::READABLE)),
            ..Default::default()
        };

        let mut buf = String::new();
        assert_eq!(transport.read_to_string(&mut buf).await.unwrap(), 17);
        assert_eq!(buf, "hello world, test");
    }

    #[test(tokio::test)]
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

    #[test(tokio::test)]
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

    #[test(tokio::test)]
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

    #[test(tokio::test)]
    async fn write_all_should_continue_to_call_try_write_while_it_returns_would_block() {
        // Configure `try_write` to alternate between writing a byte and WouldBlock
        let transport = TestTransport {
            f_try_write: Box::new(|buf| {
                static mut CNT: u8 = 0;
                unsafe {
                    if CNT.is_multiple_of(2) {
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

    #[test(tokio::test)]
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
