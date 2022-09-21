use super::{Interest, Ready, Reconnectable, Transport};
use crate::utils;
use async_trait::async_trait;
use bytes::{Buf, BytesMut};
use log::*;
use serde::{Deserialize, Serialize};
use std::{fmt, io};

mod codec;
mod exchange;
mod frame;
mod handshake;

pub use codec::*;
pub use exchange::*;
pub use frame::*;
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
    incoming: BytesMut,
    outgoing: BytesMut,
}

impl<T, const CAPACITY: usize> FramedTransport<T, CAPACITY> {
    pub fn new(inner: T, codec: BoxedCodec) -> Self {
        Self {
            inner,
            codec,
            incoming: BytesMut::with_capacity(CAPACITY),
            outgoing: BytesMut::with_capacity(CAPACITY),
        }
    }

    /// Creates a new [`FramedTransport`] using the [`PlainCodec`]
    pub fn plain(inner: T) -> Self {
        Self::new(inner, Box::new(PlainCodec::new()))
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

    /// Perform the client-side of a handshake. See [`handshake`] for more details.
    ///
    /// [`handshake`]: FramedTransport::handshake
    pub async fn client_handshake(&mut self) -> io::Result<()> {
        self.handshake(Handshake::client()).await
    }

    /// Perform the server-side of a handshake. See [`handshake`] for more details.
    ///
    /// [`handshake`]: FramedTransport::handshake
    pub async fn server_handshake(&mut self) -> io::Result<()> {
        self.handshake(Handshake::server()).await
    }

    /// Performs a handshake in order to establish a new codec to use between this transport and
    /// the other side. The parameter `handshake` defines how the transport will handle the
    /// handshake with `Client` being used to pick the compression and encryption used while
    /// `Server` defines what the choices are for compression and encryption.
    ///
    /// This will reset the framed transport's codec to [`PlainCodec`] in order to communicate
    /// which compression and encryption to use. Upon selecting an encryption type, a shared secret
    /// key will be derived on both sides and used to establish the [`EncryptionCodec`], which in
    /// combination with the [`CompressionCodec`] (if any) will replace this transport's codec.
    ///
    /// ### Client
    ///
    /// 1. Wait for options from server
    /// 2. Send to server a compression and encryption choice
    /// 3. Configure framed transport using selected choices
    /// 4. Invoke on_handshake function
    ///
    /// ### Server
    ///
    /// 1. Send options to client
    /// 2. Receive choices from client
    /// 3. Configure framed transport using client's choices
    /// 4. Invoke on_handshake function
    ///
    /// ### Failure
    ///
    /// The handshake will fail in several cases:
    ///
    /// * If any frame during the handshake fails to be serialized
    /// * If any unexpected frame is received during the handshake
    /// * If using encryption and unable to derive a shared secret key
    ///
    /// If a failure happens, the codec will be reset to what it was prior to the handshake
    /// request, and all internal buffers will be cleared to avoid corruption.
    ///
    pub async fn handshake(&mut self, handshake: Handshake) -> io::Result<()> {
        // Place transport in plain text communication mode for start of handshake, and clear any
        // data that is lingering within internal buffers
        //
        // NOTE: We grab the old codec in case we encounter an error and need to reset it
        let old_codec = std::mem::replace(&mut self.codec, Box::new(PlainCodec::new()));
        self.clear();

        // Transform the transport's codec to abide by the choice. In the case of an error, we
        // reset the codec back to what it was prior to attempting the handshake and clear the
        // internal buffers as they may be corrupt.
        match self.handshake_impl(handshake).await {
            Ok(codec) => {
                self.set_codec(codec);
                Ok(())
            }
            Err(x) => {
                self.set_codec(old_codec);
                self.clear();
                Err(x)
            }
        }
    }

    async fn handshake_impl(&mut self, handshake: Handshake) -> io::Result<BoxedCodec> {
        #[derive(Debug, Serialize, Deserialize)]
        struct Choice {
            compression_level: Option<CompressionLevel>,
            compression_type: Option<CompressionType>,
            encryption_type: Option<EncryptionType>,
        }

        #[derive(Debug, Serialize, Deserialize)]
        struct Options {
            compression_types: Vec<CompressionType>,
            encryption_types: Vec<EncryptionType>,
        }

        macro_rules! write_frame {
            ($data:expr) => {{
                self.write_frame(utils::serialize_to_vec(&$data)?).await?
            }};
        }

        macro_rules! next_frame_as {
            ($type:ty) => {{
                let frame = self.read_frame().await?.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "Transport closed early")
                })?;

                utils::deserialize_from_slice::<$type>(frame.as_item())?
            }};
        }

        // Determine compression and encryption to apply to framed transport
        let choice = match handshake {
            Handshake::Client {
                preferred_compression_type,
                preferred_compression_level,
                preferred_encryption_type,
            } => {
                // Receive options from the server and pick one
                debug!("[Handshake] Client waiting on server options");
                let options = next_frame_as!(Options);

                // Choose a compression and encryption option from the options
                debug!("[Handshake] Client selecting from server options: {options:#?}");
                let choice = Choice {
                    // Use preferred compression if available, otherwise default to no compression
                    // to avoid choosing something poor
                    compression_type: preferred_compression_type
                        .filter(|ty| options.compression_types.contains(ty)),

                    // Use preferred compression level, otherwise allowing the server to pick
                    compression_level: preferred_compression_level,

                    // Use preferred encryption, otherwise pick first non-unknown encryption type
                    // that is available instead
                    encryption_type: preferred_encryption_type
                        .filter(|ty| options.encryption_types.contains(ty))
                        .or_else(|| {
                            options
                                .encryption_types
                                .iter()
                                .find(|ty| !ty.is_unknown())
                                .copied()
                        }),
                };

                // Report back to the server the choice
                debug!("[Handshake] Client reporting choice: {choice:#?}");
                write_frame!(choice);

                choice
            }
            Handshake::Server {
                compression_types,
                encryption_types,
            } => {
                let options = Options {
                    compression_types: compression_types.to_vec(),
                    encryption_types: encryption_types.to_vec(),
                };

                // Send options to the client
                debug!("[Handshake] Server sending options: {options:#?}");
                write_frame!(options);

                // Get client's response with selected compression and encryption
                debug!("[Handshake] Server waiting on client choice");
                next_frame_as!(Choice)
            }
        };

        debug!("[Handshake] Building compression & encryption codecs based on {choice:#?}");
        let compression_level = choice.compression_level.unwrap_or_default();

        // Acquire a codec for the compression type
        let compression_codec = choice
            .compression_type
            .map(|ty| ty.new_codec(compression_level))
            .transpose()?;

        // In the case that we are using encryption, we derive a shared secret key to use with the
        // encryption type
        let encryption_codec = match choice.encryption_type {
            Some(ty) => {
                #[derive(Serialize, Deserialize)]
                struct KeyExchangeData {
                    /// Bytes of the public key
                    #[serde(with = "serde_bytes")]
                    public_key: PublicKeyBytes,

                    /// Randomly generated salt
                    #[serde(with = "serde_bytes")]
                    salt: Salt,
                }

                let exchange = KeyExchange::default();
                write_frame!(KeyExchangeData {
                    public_key: exchange.pk_bytes(),
                    salt: *exchange.salt(),
                });

                // TODO: This key only works because it happens to be 32 bytes and our encryption
                //       also wants a 32-byte key. Once we introduce new encryption algorithms that
                //       are not using 32-byte keys, the key exchange will need to support deriving
                //       other length keys.
                let data = next_frame_as!(KeyExchangeData);
                let key = exchange.derive_shared_secret(data.public_key, data.salt)?;
                Some(ty.new_codec(key.unprotected_as_bytes())?)
            }
            None => None,
        };

        // Bundle our compression and encryption codecs into a single, chained codec
        let codec: BoxedCodec = match (compression_codec, encryption_codec) {
            // If we have both encryption and compression, do the encryption first and then
            // compress in order to get smallest result
            (Some(c), Some(e)) => Box::new(ChainCodec::new(e, c)),

            // If we just have compression, pass along the compression codec
            (Some(c), None) => Box::new(c),

            // If we just have encryption, pass along the encryption codec
            (None, Some(e)) => Box::new(e),

            // If we have neither compression nor encryption, use a plaintext codec
            (None, None) => Box::new(PlainCodec::new()),
        };

        Ok(codec)
    }
}

#[async_trait]
impl<T, const CAPACITY: usize> Reconnectable for FramedTransport<T, CAPACITY>
where
    T: Transport + Send + Sync,
{
    async fn reconnect(&mut self) -> io::Result<()> {
        Reconnectable::reconnect(&mut self.inner).await
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
        let a = FramedTransport::new(a, Box::new(PlainCodec::new()));
        let b = FramedTransport::new(b, Box::new(PlainCodec::new()));
        (a, b)
    }
}

#[cfg(test)]
impl FramedTransport<super::InmemoryTransport> {
    /// Generates a test pair with default capacity
    pub fn test_pair(
        buffer: usize,
    ) -> (
        FramedTransport<super::InmemoryTransport>,
        FramedTransport<super::InmemoryTransport>,
    ) {
        Self::pair(buffer)
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
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_read: Box::new(|_| Err(io::Error::from(io::ErrorKind::WouldBlock))),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
        );
        assert_eq!(
            transport.try_read_frame().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );

        // Should fail if not read enough bytes before blocking
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_read: simulate_try_read(vec![Frame::new(b"some data")], 1, |cnt| cnt == 1),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
        );
        assert_eq!(
            transport.try_read_frame().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn try_read_frame_should_return_error_if_encountered_error_with_reading_bytes() {
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_read: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
        );
        assert_eq!(
            transport.try_read_frame().unwrap_err().kind(),
            io::ErrorKind::NotConnected
        );
    }

    #[test]
    fn try_read_frame_should_return_error_if_encountered_error_during_decode() {
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_read: simulate_try_read(vec![Frame::new(b"some data")], 1, |_| false),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            Box::new(ErrCodec),
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

        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_read: Box::new(move |buf| {
                    buf[..data.len()].copy_from_slice(data.as_ref());
                    Ok(data.len())
                }),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
        );
        assert_eq!(transport.try_read_frame().unwrap().unwrap(), b"hello world");
    }

    #[test]
    fn try_read_frame_should_keep_reading_until_a_frame_is_found() {
        const STEP_SIZE: usize = Frame::HEADER_SIZE + 7;

        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_read: simulate_try_read(
                    vec![Frame::new(b"hello world"), Frame::new(b"test hello")],
                    STEP_SIZE,
                    |_| false,
                ),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
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
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::WouldBlock))),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
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
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
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
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_write: Box::new(|buf| Ok(buf.len())),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            Box::new(ErrCodec),
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
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_write: Box::new(move |buf| {
                    let len = buf.len();
                    tx.send(buf.to_vec()).unwrap();
                    Ok(len)
                }),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
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
        let mut transport = FramedTransport::<_>::new(
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
            Box::new(OkCodec),
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
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
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
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_write: Box::new(|_| Ok(0)),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
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
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_write: Box::new(|_| Err(io::Error::from(io::ErrorKind::NotConnected))),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
        );

        // Perform flush and verify nothing happens
        transport.try_flush().unwrap();
    }

    #[test]
    fn try_flush_should_continually_call_try_write_until_outgoing_buffer_is_empty() {
        const STEP_SIZE: usize = 5;
        let (tx, rx) = std::sync::mpsc::sync_channel(10);
        let mut transport = FramedTransport::<_>::new(
            TestTransport {
                f_try_write: Box::new(move |buf| {
                    let len = std::cmp::min(STEP_SIZE, buf.len());
                    tx.send(buf[..len].to_vec()).unwrap();
                    Ok(len)
                }),
                f_ready: Box::new(|_| Ok(Ready::WRITABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
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
