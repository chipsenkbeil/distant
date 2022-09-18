use super::{Interest, Ready, Reconnectable, Transport};
use async_trait::async_trait;
use bytes::{Buf, BytesMut};
use std::{fmt, io, sync::Arc};

mod codec;
pub use codec::*;

mod frame;
pub use frame::*;

mod handshake;
pub use handshake::*;

/// By default, framed transport's initial capacity (and max single-read) will be 8 KiB
const DEFAULT_CAPACITY: usize = 8 * 1024;

/// Represents a wrapper around a [`Transport`] that reads and writes using frames defined by a
/// [`Codec`]. `CAPACITY` represents both the initial capacity of incoming and outgoing buffers as
/// well as the maximum bytes read per call to [`try_read`].
///
/// [`try_read`]: Transport::try_read
#[derive(Clone)]
pub struct FramedTransport<T, const CAPACITY: usize = DEFAULT_CAPACITY> {
    inner: T,
    codec: BoxedCodec,
    handshake: Handshake,

    incoming: BytesMut,
    outgoing: BytesMut,
}

impl<T, const CAPACITY: usize> FramedTransport<T, CAPACITY> {
    fn new(inner: T, codec: BoxedCodec, handshake: Handshake) -> Self {
        Self {
            inner,
            codec,
            handshake,
            incoming: BytesMut::with_capacity(CAPACITY),
            outgoing: BytesMut::with_capacity(CAPACITY),
        }
    }

    /// Creates a new [`FramedTransport`] using the [`PlainCodec`]
    fn plain(inner: T, handshake: Handshake) -> Self {
        Self::new(inner, Box::new(PlainCodec::new()), handshake)
    }

    /// Performs a handshake with the other side of the `transport` in order to determine which
    /// [`Codec`] to use as well as perform any additional logic to prepare the framed transport.
    ///
    /// Will use the handshake criteria provided in `handshake`
    pub async fn from_handshake(
        transport: T,
        handshake: Handshake,
    ) -> io::Result<FramedTransport<T, CAPACITY>>
    where
        T: Transport,
    {
        let mut transport = Self::plain(transport, handshake);
        handshake::do_handshake(&mut transport).await?;
        Ok(transport)
    }

    /// Replaces the current codec with the provided codec. Note that any bytes in the incoming or
    /// outgoing buffers will remain in the transport, meaning that this can cause corruption if
    /// the bytes in the buffers do not match the new codec.
    ///
    /// For safety, use [`clear`] to wipe the buffers before further use.
    ///
    /// [`clear`]: FramedTransport::clear
    pub fn set_codec(&mut self, codec: BoxedCodec) {
        self.codec = codec;
    }

    /// Clears the internal buffers used by the transport
    pub fn clear(&mut self) {
        self.incoming.clear();
        self.outgoing.clear();
    }
}

impl<T, const CAPACITY: usize> fmt::Debug for FramedTransport<T, CAPACITY> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FramedTransport")
            .field("capacity", &CAPACITY)
            .field("incoming", &self.incoming)
            .field("outgoing", &self.outgoing)
            .finish()
    }
}

impl<T, const CAPACITY: usize> FramedTransport<T, CAPACITY>
where
    T: Transport,
{
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
}

impl<T, const CAPACITY: usize> FramedTransport<T, CAPACITY>
where
    T: Transport,
{
    /// Reads a frame of bytes by using the [`Codec`] tied to this transport. Returns
    /// `Ok(Some(frame))` upon reading a frame, or `Ok(None)` if the underlying transport has
    /// closed.
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to read data or has not received a full frame before waiting.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_read_frame(&mut self) -> io::Result<Option<OwnedFrame>> {
        // Continually read bytes into the incoming queue and then attempt to tease out a frame
        let mut buf = [0; CAPACITY];

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

                    // Attempt to read a frame, returning the decoded frame if we get one,
                    // continuing to try to read more bytes if we don't find a frame, and returning
                    // any error that is encountered from reading frames or failing to decode
                    let frame = match Frame::read(&mut self.incoming) {
                        Ok(Some(frame)) => frame,
                        Ok(None) => continue,
                        Err(x) => return Err(x),
                    };

                    return Ok(Some(self.codec.decode(frame)?.into_owned()));
                }

                // Any error (including WouldBlock) will get bubbled up
                Err(x) => return Err(x),
            }
        }
    }

    /// Continues to invoke [`try_read_frame`] until a frame is successfully read, an error is
    /// encountered that is not [`ErrorKind::WouldBlock`], or the underlying transport has closed.
    ///
    /// [`try_read_frame`]: FramedTransport::try_read_frame
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub async fn read_frame(&mut self) -> io::Result<Option<OwnedFrame>> {
        loop {
            self.readable().await?;

            match self.try_read_frame() {
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => continue,
                x => return x,
            }
        }
    }

    /// Writes a `frame` of bytes by using the [`Codec`] tied to this transport.
    ///
    /// This is accomplished by continually calling the inner transport's `try_write`. If 0 is
    /// returned from a call to `try_write`, this will fail with [`ErrorKind::WriteZero`].
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to write data or has not written the entire frame before waiting.
    ///
    /// [`ErrorKind::WriteZero`]: io::ErrorKind::WriteZero
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_write_frame<'a>(&mut self, frame: impl Into<Frame<'a>>) -> io::Result<()> {
        // Encode the frame and store it in our outgoing queue
        let frame = self.codec.encode(frame.into())?;
        frame.write(&mut self.outgoing)?;

        // Attempt to write everything in our queue
        self.try_flush()
    }

    /// Invokes [`try_write_frame`] followed by a continuous calls to [`try_flush`] until a frame
    /// is successfully written, an error is encountered that is not [`ErrorKind::WouldBlock`], or
    /// the underlying transport has closed.
    ///
    /// [`try_write_frame`]: FramedTransport::try_write_frame
    /// [`try_flush`]: FramedTransport::try_flush
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub async fn write_frame<'a>(&mut self, frame: impl Into<Frame<'a>>) -> io::Result<()> {
        self.writeable().await?;

        match self.try_write_frame(frame) {
            // Would block, so continually try to flush until good to go
            Err(x) if x.kind() == io::ErrorKind::WouldBlock => loop {
                self.writeable().await?;
                match self.try_flush() {
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => continue,
                    x => return x,
                }
            },

            // Already fully succeeded or failed
            x => x,
        }
    }
}

#[async_trait]
impl<T, const CAPACITY: usize> Reconnectable for FramedTransport<T, CAPACITY>
where
    T: Transport + Send + Sync,
{
    async fn reconnect(&mut self) -> io::Result<()> {
        // Establish a new connection
        Reconnectable::reconnect(&mut self.inner).await?;

        // Perform handshake again, which can result in the underlying codec
        // changing based on the exchange; so, we want to clear out any lingering
        // bytes in the incoming and outgoing queues
        self.clear();
        handshake::do_handshake(self).await
    }
}

impl<const CAPACITY: usize> FramedTransport<super::InmemoryTransport, CAPACITY> {
    /// Produces a pair of inmemory transports that are connected to each other using
    /// a standard codec
    ///
    /// Sets the buffer for message passing for each underlying transport to the given buffer size
    pub fn pair(
        buffer: usize,
    ) -> (
        FramedTransport<super::InmemoryTransport, CAPACITY>,
        FramedTransport<super::InmemoryTransport, CAPACITY>,
    ) {
        let (a, b) = super::InmemoryTransport::pair(buffer);
        let a = FramedTransport::new(
            a,
            Box::new(PlainCodec::new()),
            Handshake::Client {
                key: HeapSecretKey::from(Vec::new()),
                preferred_compression_type: None,
                preferred_compression_level: None,
                preferred_encryption_type: None,
            },
        );
        let b = FramedTransport::new(
            b,
            Box::new(PlainCodec::new()),
            Handshake::Server {
                key: HeapSecretKey::from(Vec::new()),
                compression_types: Vec::new(),
                encryption_types: Vec::new(),
            },
        );
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestTransport;
    use bytes::BufMut;

    /// Codec that always succeeds without altering the frame
    #[derive(Clone)]
    struct OkCodec;

    impl Codec for OkCodec {
        fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
            Ok(frame)
        }

        fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
            Ok(frame)
        }
    }

    /// Codec that always fails
    #[derive(Clone)]
    struct ErrCodec;

    impl Codec for ErrCodec {
        fn encode<'a>(&mut self, _frame: Frame<'a>) -> io::Result<Frame<'a>> {
            Err(io::Error::from(io::ErrorKind::Other))
        }

        fn decode<'a>(&mut self, _frame: Frame<'a>) -> io::Result<Frame<'a>> {
            Err(io::Error::from(io::ErrorKind::Other))
        }
    }

    /// Simulate calls to try_read by feeding back `data` in `step` increments, triggering a block
    /// if `block_on` returns true where `block_on` is provided a counter value that is incremented
    /// every time the simulated `try_read` function is called
    ///
    /// NOTE: This will inject the frame len in front of the provided data to properly simulate
    ///       receiving a frame of data
    fn simulate_try_read(
        frames: Vec<Frame>,
        step: usize,
        block_on: impl Fn(usize) -> bool + Send + Sync + 'static,
    ) -> Box<dyn Fn(&mut [u8]) -> io::Result<usize> + Send + Sync> {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Stuff all of our frames into a single byte collection
        let data = {
            let mut buf = BytesMut::new();

            for frame in frames {
                frame.write(&mut buf).unwrap();
            }

            buf.to_vec()
        };

        let idx = AtomicUsize::new(0);
        let cnt = AtomicUsize::new(0);

        Box::new(move |buf| {
            if block_on(cnt.fetch_add(1, Ordering::Relaxed)) {
                return Err(io::Error::from(io::ErrorKind::WouldBlock));
            }

            let start = idx.fetch_add(step, Ordering::Relaxed);
            let end = start + step;
            let end = if end > data.len() { data.len() } else { end };
            let len = if start > end { 0 } else { end - start };

            buf[..len].copy_from_slice(&data[start..end]);
            Ok(len)
        })
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
            OkCodec,
        );
        assert_eq!(
            transport.try_read_frame().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );

        // Should fail if not read enough bytes before blocking
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_read: simulate_try_read(vec![Frame::new(b"some data")], 1, |cnt| cnt == 1),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            OkCodec,
        );
        assert_eq!(
            transport.try_read_frame().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn try_read_frame_should_return_error_if_encountered_error_with_reading_bytes() {
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_read: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            OkCodec,
        );
        assert_eq!(
            transport.try_read_frame().unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
    }

    #[test]
    fn try_read_frame_should_return_error_if_encountered_error_during_decode() {
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_read: simulate_try_read(vec![Frame::new(b"some data")], 1, |_| false),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            ErrCodec,
        );
        assert_eq!(
            transport.try_read_frame().unwrap_err().kind(),
            io::ErrorKind::Other
        );
    }

    #[test]
    fn try_read_frame_should_return_next_available_frame() {
        let data = {
            let mut data = BytesMut::new();
            Frame::new(b"hello world").write(&mut data).unwrap();
            data.freeze()
        };

        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_read: Box::new(move |buf| {
                    buf[..data.len()].copy_from_slice(data.as_ref());
                    Ok(data.len())
                }),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            OkCodec,
        );
        assert_eq!(transport.try_read_frame().unwrap().unwrap(), b"hello world");
    }

    #[test]
    fn try_read_frame_should_keep_reading_until_a_frame_is_found() {
        const STEP_SIZE: usize = Frame::HEADER_SIZE + 7;

        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_read: simulate_try_read(
                    vec![Frame::new(b"hello world"), Frame::new(b"test hello")],
                    STEP_SIZE,
                    |_| false,
                ),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            OkCodec,
        );
        assert_eq!(transport.try_read_frame().unwrap().unwrap(), b"hello world");

        // Should have leftover bytes from next frame
        // where len = 10, "tes"
        assert_eq!(
            transport.incoming.to_vec(),
            [0, 0, 0, 0, 0, 0, 0, 10, b't', b'e', b's']
        );
    }

    #[test]
    fn try_write_frame_should_return_would_block_if_fails_to_write_frame_before_blocking() {
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::WouldBlock))),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            OkCodec,
        );

        // First call will only write part of the frame and then return WouldBlock
        assert_eq!(
            transport
                .try_write_frame(b"hello world")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn try_write_frame_should_return_error_if_encountered_error_with_writing_bytes() {
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            OkCodec,
        );
        assert_eq!(
            transport
                .try_write_frame(b"hello world")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotConnected
        );
    }

    #[test]
    fn try_write_frame_should_return_error_if_encountered_error_during_encode() {
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_write: Box::new(|buf| Ok(buf.len())),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            ErrCodec,
        );
        assert_eq!(
            transport
                .try_write_frame(b"hello world")
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );
    }

    #[test]
    fn try_write_frame_should_write_entire_frame_if_possible() {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_write: Box::new(move |buf| {
                    let len = buf.len();
                    tx.send(buf.to_vec()).unwrap();
                    Ok(len)
                }),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            OkCodec,
        );

        transport.try_write_frame(b"hello world").unwrap();

        // Transmitted data should be encoded using the framed transport's codec
        assert_eq!(
            rx.try_recv().unwrap(),
            [11u64.to_be_bytes().as_slice(), b"hello world".as_slice()].concat()
        );
    }

    #[test]
    fn try_write_frame_should_write_any_prior_queued_bytes_before_writing_next_frame() {
        const STEP_SIZE: usize = Frame::HEADER_SIZE + 5;
        let (tx, rx) = std::sync::mpsc::sync_channel(10);
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_write: Box::new(move |buf| {
                    static mut CNT: usize = 0;
                    unsafe {
                        CNT += 1;
                        if CNT == 2 {
                            Err(io::Error::from(io::ErrorKind::WouldBlock))
                        } else {
                            let len = std::cmp::min(STEP_SIZE, buf.len());
                            tx.send(buf[..len].to_vec()).unwrap();
                            Ok(len)
                        }
                    }
                }),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            OkCodec,
        );

        // First call will only write part of the frame and then return WouldBlock
        assert_eq!(
            transport
                .try_write_frame(b"hello world")
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );

        // Transmitted data should be encoded using the framed transport's codec
        assert_eq!(
            rx.try_recv().unwrap(),
            [11u64.to_be_bytes().as_slice(), b"hello".as_slice()].concat()
        );
        assert_eq!(
            rx.try_recv().unwrap_err(),
            std::sync::mpsc::TryRecvError::Empty
        );

        // Next call will keep writing successfully until done
        transport.try_write_frame(b"test").unwrap();
        assert_eq!(
            rx.try_recv().unwrap(),
            [b' ', b'w', b'o', b'r', b'l', b'd', 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(rx.try_recv().unwrap(), [4, b't', b'e', b's', b't']);
        assert_eq!(
            rx.try_recv().unwrap_err(),
            std::sync::mpsc::TryRecvError::Empty
        );
    }

    #[test]
    fn try_flush_should_return_error_if_try_write_fails() {
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            OkCodec,
        );

        // Set our outgoing buffer to flush
        transport.outgoing.put_slice(b"hello world");

        // Perform flush and verify error happens
        assert_eq!(
            transport.try_flush().unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
    }

    #[test]
    fn try_flush_should_return_error_if_try_write_returns_0_bytes_written() {
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_write: Box::new(|_| Ok(0)),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            OkCodec,
        );

        // Set our outgoing buffer to flush
        transport.outgoing.put_slice(b"hello world");

        // Perform flush and verify error happens
        assert_eq!(
            transport.try_flush().unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
    }

    #[test]
    fn try_flush_should_be_noop_if_nothing_to_flush() {
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            OkCodec,
        );

        // Perform flush and verify nothing happens
        transport.try_flush().unwrap();
    }

    #[test]
    fn try_flush_should_continually_call_try_write_until_outgoing_buffer_is_empty() {
        const STEP_SIZE: usize = 5;
        let (tx, rx) = std::sync::mpsc::sync_channel(10);
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_write: Box::new(move |buf| {
                    let len = std::cmp::min(STEP_SIZE, buf.len());
                    tx.send(buf[..len].to_vec()).unwrap();
                    Ok(len)
                }),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            OkCodec,
        );

        // Set our outgoing buffer to flush
        transport.outgoing.put_slice(b"hello world");

        // Perform flush
        transport.try_flush().unwrap();

        // Verify outgoing data flushed with N calls to try_write
        assert_eq!(rx.try_recv().unwrap(), b"hello".as_slice());
        assert_eq!(rx.try_recv().unwrap(), b" worl".as_slice());
        assert_eq!(rx.try_recv().unwrap(), b"d".as_slice());
        assert_eq!(
            rx.try_recv().unwrap_err(),
            std::sync::mpsc::TryRecvError::Empty
        );
    }
}
