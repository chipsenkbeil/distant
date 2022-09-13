use super::{Interest, Ready, Reconnectable, Transport};
use async_trait::async_trait;
use bytes::{Buf, BytesMut};
use std::io;

mod codec;
pub use codec::*;

/// By default, framed transport's initial capacity (and max single-read) will be 8 KiB
const DEFAULT_CAPACITY: usize = 8 * 1024;

/// Represents a wrapper around a [`Transport`] that reads and writes using frames defined by a
/// [`Codec`]
pub struct FramedTransport<T, C> {
    inner: T,
    codec: C,

    incoming: BytesMut,
    outgoing: BytesMut,
}

impl<T, C> FramedTransport<T, C>
where
    T: Transport,
    C: Codec,
{
    pub fn new(inner: T, codec: C) -> Self {
        Self {
            inner,
            codec,
            incoming: BytesMut::with_capacity(DEFAULT_CAPACITY),
            outgoing: BytesMut::with_capacity(DEFAULT_CAPACITY),
        }
    }

    /// Reads a frame of bytes by using the [`Codec`] tied to this transport. Returns
    /// `Ok(Some(frame))` upon reading a frame, or `Ok(None)` if the underlying transport has
    /// closed.
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to read data or has not received a full frame before waiting.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_read_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        // Continually read bytes into the incoming queue and then attempt to tease out a frame
        let mut buf = [0; DEFAULT_CAPACITY];

        loop {
            match self.inner.try_read(&mut buf) {
                // Getting 0 bytes on read indicates the channel has closed. If we were still
                // expecting more bytes for our frame, then this is an error, otherwise if we
                // have nothing remaining if our queue then this is an expected end and we
                // return None
                Ok(0) if self.incoming.is_empty() => return Ok(None),
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),

                // Got some additional bytes, which we will add to our queue and then attempt to
                // decode into a frame
                Ok(n) => {
                    self.incoming.extend_from_slice(&buf[..n]);

                    // Attempt to decode a frame, returning the frame if we get one, continuing to
                    // try to read more bytes if we don't find a frame, and returing any error that
                    // is encountered from the decode call
                    match self.codec.decode(&mut self.incoming) {
                        Ok(Some(frame)) => return Ok(Some(frame)),
                        Ok(None) => continue,

                        // TODO: tokio-util's decoder would cause Framed to return Ok(None)
                        //       if the decoder failed as that indicated a corrupt stream.
                        //
                        //       Should we continue mirroring this behavior?
                        Err(x) => return Err(x),
                    }
                }

                // Any error (including WouldBlock) will get bubbled up
                Err(x) => return Err(x),
            }
        }
    }

    /// Writes an `item` of bytes as a frame by using the [`Codec`] tied to this transport.
    ///
    /// This is accomplished by continually calling the inner transport's `try_write`. If 0 is
    /// returned from a call to `try_write`, this will fail with [`ErrorKind::WriteZero`].
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to write data or has not written the entire frame before waiting.
    ///
    /// [`ErrorKind::WriteZero`]: io::ErrorKind::WriteZero
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_write_frame(&mut self, item: &[u8]) -> io::Result<()> {
        // Queue up the item as a new frame of bytes
        self.codec.encode(item, &mut self.outgoing)?;

        // Attempt to write everything in our queue
        self.try_flush()
    }

    /// Attempts to flush any remaining bytes in the outgoing queue.
    ///
    /// This is accomplished by continually calling the inner transport's `try_write`. If 0 is
    /// returned from a call to `try_write`, this will fail with [`ErrorKind::WriteZero`].
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to write data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_flush(&mut self) -> io::Result<()> {
        // Continue to send from the outgoing buffer until we either finish or fail
        while !self.outgoing.is_empty() {
            match self.inner.try_write(self.outgoing.as_ref()) {
                // Getting 0 bytes on write indicates the channel has closed
                Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),

                // Successful write will advance the outgoing buffer
                Ok(n) => self.outgoing.advance(n),

                // Any error (including WouldBlock) will get bubbled up
                Err(x) => return Err(x),
            }
        }

        Ok(())
    }

    /// Waits for the transport to be ready based on the given interest, returning the ready status
    pub async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        Transport::ready(&self.inner, interest).await
    }

    /// Waits for the transport to be readable to follow up with `try_read`
    pub async fn readable(&self) -> io::Result<()> {
        let _ = self.ready(Interest::READABLE).await?;
        Ok(())
    }

    /// Waits for the transport to be writeable to follow up with `try_write`
    pub async fn writeable(&self) -> io::Result<()> {
        let _ = self.ready(Interest::WRITABLE).await?;
        Ok(())
    }
}

#[async_trait]
impl<T, C> Reconnectable for FramedTransport<T, C>
where
    T: Transport + Send,
    C: Codec + Send,
{
    async fn reconnect(&mut self) -> io::Result<()> {
        Reconnectable::reconnect(&mut self.inner).await
    }
}

impl FramedTransport<super::InmemoryTransport, PlainCodec> {
    /// Produces a pair of inmemory transports that are connected to each other using
    /// a standard codec
    ///
    /// Sets the buffer for message passing for each underlying transport to the given buffer size
    pub fn pair(
        buffer: usize,
    ) -> (
        FramedTransport<super::InmemoryTransport, PlainCodec>,
        FramedTransport<super::InmemoryTransport, PlainCodec>,
    ) {
        let (a, b) = super::InmemoryTransport::pair(buffer);
        let a = FramedTransport::new(a, PlainCodec::new());
        let b = FramedTransport::new(b, PlainCodec::new());
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestTransport;
    use bytes::BufMut;

    /// Test codec makes a frame be {len}{bytes}, where len has a max size of 255
    #[derive(Clone)]
    struct TestCodec;

    impl Codec for TestCodec {
        fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> io::Result<()> {
            dst.put_u8(item.len() as u8);
            dst.extend_from_slice(item);
            Ok(())
        }

        fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<u8>>> {
            if src.is_empty() {
                return Ok(None);
            }

            let len = src[0] as usize;
            if src.len() - 1 < len {
                return Ok(None);
            }

            let frame = src.split_to(len + 1);
            let frame = frame[1..].to_vec();
            Ok(Some(frame))
        }
    }

    #[test]
    fn try_read_frame_should_return_would_block_if_fails_to_read_frame_before_blocking() {
        // Should fail if immediately blocks
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_read: Box::new(|_| Err(io::Error::from(io::ErrorKind::WouldBlock))),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            TestCodec,
        );
        assert_eq!(
            transport.try_read_frame().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );

        // Should fail if not read enough bytes before blocking
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_read: Box::new(|buf| {
                    static mut CNT: u8 = 0;
                    unsafe {
                        CNT += 1;

                        if CNT == 2 {
                            Err(io::Error::from(io::ErrorKind::WouldBlock))
                        } else {
                            buf[0] = CNT;
                            Ok(1)
                        }
                    }
                }),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            TestCodec,
        );
        assert_eq!(
            transport.try_read_frame().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn try_read_frame_should_return_error_if_encountered_error_with_reading_bytes() {
        todo!();
    }

    #[test]
    fn try_read_frame_should_return_none_if_encountered_error_during_decode() {
        todo!();
    }

    #[test]
    fn try_read_frame_should_return_next_available_frame() {
        todo!();
    }

    #[test]
    fn try_write_frame_should_return_would_block_if_fails_to_write_frame_before_blocking() {
        todo!();
    }

    #[test]
    fn try_write_frame_should_return_error_if_encountered_error_with_writing_bytes() {
        todo!();
    }

    #[test]
    fn try_write_frame_should_return_error_if_encountered_error_during_encode() {
        todo!();
    }

    #[test]
    fn try_write_frame_should_write_entire_frame_if_possible() {
        todo!();
    }

    #[test]
    fn try_write_frame_should_write_any_prior_queued_bytes_before_writing_next_frame() {
        todo!();
    }
}
