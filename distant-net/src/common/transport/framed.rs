use std::future::Future;
use std::time::Duration;
use std::{fmt, io};

use async_trait::async_trait;
use bytes::{Buf, BytesMut};
use log::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{InmemoryTransport, Interest, Ready, Reconnectable, Transport};
use crate::common::utils;

mod backup;
mod codec;
mod exchange;
mod frame;
mod handshake;

pub use backup::*;
pub use codec::*;
pub use exchange::*;
pub use frame::*;
pub use handshake::*;

/// Size of the read buffer when reading bytes to construct a frame
const READ_BUF_SIZE: usize = 8 * 1024;

/// Duration to wait after WouldBlock received during looping operations like `read_frame`
const SLEEP_DURATION: Duration = Duration::from_millis(1);

/// Represents a wrapper around a [`Transport`] that reads and writes using frames defined by a
/// [`Codec`].
///
/// [`try_read`]: Transport::try_read
#[derive(Clone)]
pub struct FramedTransport<T> {
    /// Inner transport wrapped to support frames of data
    inner: T,

    /// Codec used to encoding outgoing bytes and decode incoming bytes
    codec: BoxedCodec,

    /// Bytes in queue to be read
    incoming: BytesMut,

    /// Bytes in queue to be written
    outgoing: BytesMut,

    /// Stores outgoing frames in case of transmission issues
    pub backup: Backup,
}

impl<T> FramedTransport<T> {
    pub fn new(inner: T, codec: BoxedCodec) -> Self {
        Self {
            inner,
            codec,
            incoming: BytesMut::with_capacity(READ_BUF_SIZE * 2),
            outgoing: BytesMut::with_capacity(READ_BUF_SIZE * 2),
            backup: Backup::new(),
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

    /// Returns a reference to the codec used by the transport.
    ///
    /// ### Note
    ///
    /// Be careful when accessing the codec to avoid corrupting it through unexpected modifications
    /// as this will place the transport in an undefined state.
    pub fn codec(&self) -> &dyn Codec {
        self.codec.as_ref()
    }

    /// Returns a mutable reference to the codec used by the transport.
    ///
    /// ### Note
    ///
    /// Be careful when accessing the codec to avoid corrupting it through unexpected modifications
    /// as this will place the transport in an undefined state.
    pub fn mut_codec(&mut self) -> &mut dyn Codec {
        self.codec.as_mut()
    }

    /// Clears the internal transport buffers.
    pub fn clear(&mut self) {
        self.incoming.clear();
        self.outgoing.clear();
    }

    /// Returns a reference to the inner value this transport wraps.
    pub fn as_inner(&self) -> &T {
        &self.inner
    }

    /// Returns a mutable reference to the inner value this transport wraps.
    pub fn as_mut_inner(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Consumes this transport, returning the inner value that it wraps.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> fmt::Debug for FramedTransport<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FramedTransport")
            .field("incoming", &self.incoming)
            .field("outgoing", &self.outgoing)
            .field("backup", &self.backup)
            .finish()
    }
}

impl<T: Transport + 'static> FramedTransport<T> {
    /// Converts this instance to a [`FramedTransport`] whose inner [`Transport`] is [`Box`]ed.
    pub fn into_boxed(self) -> FramedTransport<Box<dyn Transport>> {
        FramedTransport {
            inner: Box::new(self.inner),
            codec: self.codec,
            incoming: self.incoming,
            outgoing: self.outgoing,
            backup: self.backup,
        }
    }
}

impl<T: Transport> FramedTransport<T> {
    /// Waits for the transport to be ready based on the given interest, returning the ready status
    pub async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        // If interest includes reading, we check if we already have a frame in our queue,
        // as there can be a scenario where a frame was received and then the connection
        // was closed, and we still want to be able to read the next frame is if it is
        // available in the connection.
        let ready = if interest.is_readable() && Frame::available(&self.incoming) {
            Ready::READABLE
        } else {
            Ready::EMPTY
        };

        // If we know that we are readable and not checking for write status, we can short-circuit
        // to avoid an async call by returning immediately that we are readable
        if !interest.is_writable() && ready.is_readable() {
            return Ok(ready);
        }

        // Otherwise, we need to check the status using the underlying transport and merge it with
        // our current understanding based on internal state
        Transport::ready(&self.inner, interest)
            .await
            .map(|r| r | ready)
    }

    /// Waits for the transport to be readable to follow up with [`try_read_frame`].
    ///
    /// [`try_read_frame`]: FramedTransport::try_read_frame
    pub async fn readable(&self) -> io::Result<()> {
        let _ = self.ready(Interest::READABLE).await?;
        Ok(())
    }

    /// Waits for the transport to be writeable to follow up with [`try_write_frame`].
    ///
    /// [`try_write_frame`]: FramedTransport::try_write_frame
    pub async fn writeable(&self) -> io::Result<()> {
        let _ = self.ready(Interest::WRITABLE).await?;
        Ok(())
    }

    /// Waits for the transport to be readable or writeable, returning the [`Ready`] status.
    pub async fn readable_or_writeable(&self) -> io::Result<Ready> {
        self.ready(Interest::READABLE | Interest::WRITABLE).await
    }

    /// Attempts to flush any remaining bytes in the outgoing queue, returning the total bytes
    /// written as a result of the flush. Note that a return of 0 bytes does not indicate that the
    /// underlying transport has closed, but rather that no bytes were flushed such as when the
    /// outgoing queue is empty.
    ///
    /// This is accomplished by continually calling the inner transport's `try_write`. If 0 is
    /// returned from a call to `try_write`, this will fail with [`ErrorKind::WriteZero`].
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to write data.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_flush(&mut self) -> io::Result<usize> {
        let mut bytes_written = 0;

        // Continue to send from the outgoing buffer until we either finish or fail
        while !self.outgoing.is_empty() {
            match self.inner.try_write(self.outgoing.as_ref()) {
                // Getting 0 bytes on write indicates the channel has closed
                Ok(0) => return Err(io::Error::from(io::ErrorKind::WriteZero)),

                // Successful write will advance the outgoing buffer
                Ok(n) => {
                    self.outgoing.advance(n);
                    bytes_written += n;
                }

                // Any error (including WouldBlock) will get bubbled up
                Err(x) => return Err(x),
            }
        }

        Ok(bytes_written)
    }

    /// Flushes all buffered, outgoing bytes using repeated calls to [`try_flush`].
    ///
    /// [`try_flush`]: FramedTransport::try_flush
    pub async fn flush(&mut self) -> io::Result<()> {
        while !self.outgoing.is_empty() {
            self.writeable().await?;
            match self.try_flush() {
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                    // NOTE: We sleep for a little bit before trying again to avoid pegging CPU
                    tokio::time::sleep(SLEEP_DURATION).await
                }
                Err(x) => return Err(x),
                Ok(_) => return Ok(()),
            }
        }

        Ok(())
    }

    /// Reads a frame of bytes by using the [`Codec`] tied to this transport. Returns
    /// `Ok(Some(frame))` upon reading a frame, or `Ok(None)` if the underlying transport has
    /// closed.
    ///
    /// This call may return an error with [`ErrorKind::WouldBlock`] in the case that the transport
    /// is not ready to read data or has not received a full frame before waiting.
    ///
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub fn try_read_frame(&mut self) -> io::Result<Option<OwnedFrame>> {
        // Attempt to read a frame, returning the decoded frame if we get one, returning any error
        // that is encountered from reading frames or failing to decode, or otherwise doing nothing
        // and continuing forward.
        macro_rules! read_next_frame {
            () => {{
                match Frame::read(&mut self.incoming) {
                    None => (),
                    Some(frame) => {
                        if frame.is_nonempty() {
                            self.backup.increment_received_cnt();
                        }
                        return Ok(Some(self.codec.decode(frame)?.into_owned()));
                    }
                }
            }};
        }

        // If we have data remaining in the buffer, we first try to parse it in case we received
        // multiple frames from a previous call.
        //
        // NOTE: This exists to avoid the situation where there is a valid frame remaining in the
        //       incoming buffer, but it is never evaluated because a call to `try_read` returns
        //       `WouldBlock`, 0 bytes, or some other error.
        if !self.incoming.is_empty() {
            read_next_frame!();
        }

        // Continually read bytes into the incoming queue and then attempt to tease out a frame
        let mut buf = [0; READ_BUF_SIZE];

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
                    read_next_frame!();
                }

                // Any error (including WouldBlock) will get bubbled up
                Err(x) => return Err(x),
            }
        }
    }

    /// Reads a frame using [`try_read_frame`] and then deserializes the bytes into `D`.
    ///
    /// [`try_read_frame`]: FramedTransport::try_read_frame
    pub fn try_read_frame_as<D: DeserializeOwned>(&mut self) -> io::Result<Option<D>> {
        match self.try_read_frame() {
            Ok(Some(frame)) => Ok(Some(utils::deserialize_from_slice(frame.as_item())?)),
            Ok(None) => Ok(None),
            Err(x) => Err(x),
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
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                    // NOTE: We sleep for a little bit before trying again to avoid pegging CPU
                    tokio::time::sleep(SLEEP_DURATION).await
                }
                x => return x,
            }
        }
    }

    /// Reads a frame using [`read_frame`] and then deserializes the bytes into `D`.
    ///
    /// [`read_frame`]: FramedTransport::read_frame
    pub async fn read_frame_as<D: DeserializeOwned>(&mut self) -> io::Result<Option<D>> {
        match self.read_frame().await {
            Ok(Some(frame)) => Ok(Some(utils::deserialize_from_slice(frame.as_item())?)),
            Ok(None) => Ok(None),
            Err(x) => Err(x),
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
    pub fn try_write_frame<'a, F>(&mut self, frame: F) -> io::Result<()>
    where
        F: TryInto<Frame<'a>>,
        F::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        // Grab the frame to send
        let frame = frame
            .try_into()
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?;

        // Encode the frame and store it in our outgoing queue
        self.codec
            .encode(frame.as_borrowed())?
            .write(&mut self.outgoing);

        // Update tracking stats and more of backup if frame is nonempty
        if frame.is_nonempty() {
            // Once the frame enters our queue, we count it as written, even if it isn't fully flushed
            self.backup.increment_sent_cnt();

            // Then we store the raw frame (non-encoded) for the future in case we need to retry
            // sending it later (possibly with a different codec)
            self.backup.push_frame(frame);
        }

        // Attempt to write everything in our queue
        self.try_flush()?;

        Ok(())
    }

    /// Serializes `value` into bytes and passes them to [`try_write_frame`].
    ///
    /// [`try_write_frame`]: FramedTransport::try_write_frame
    pub fn try_write_frame_for<D: Serialize>(&mut self, value: &D) -> io::Result<()> {
        let data = utils::serialize_to_vec(value)?;
        self.try_write_frame(data)
    }

    /// Invokes [`try_write_frame`] followed by a continuous calls to [`try_flush`] until a frame
    /// is successfully written, an error is encountered that is not [`ErrorKind::WouldBlock`], or
    /// the underlying transport has closed.
    ///
    /// [`try_write_frame`]: FramedTransport::try_write_frame
    /// [`try_flush`]: FramedTransport::try_flush
    /// [`ErrorKind::WouldBlock`]: io::ErrorKind::WouldBlock
    pub async fn write_frame<'a, F>(&mut self, frame: F) -> io::Result<()>
    where
        F: TryInto<Frame<'a>>,
        F::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        self.writeable().await?;

        match self.try_write_frame(frame) {
            // Would block, so continually try to flush until good to go
            Err(x) if x.kind() == io::ErrorKind::WouldBlock => loop {
                self.writeable().await?;
                match self.try_flush() {
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                        // NOTE: We sleep for a little bit before trying again to avoid pegging CPU
                        tokio::time::sleep(SLEEP_DURATION).await
                    }
                    Err(x) => return Err(x),
                    Ok(_) => return Ok(()),
                }
            },

            // Already fully succeeded or failed
            x => x,
        }
    }

    /// Serializes `value` into bytes and passes them to [`write_frame`].
    ///
    /// [`write_frame`]: FramedTransport::write_frame
    pub async fn write_frame_for<D: Serialize>(&mut self, value: &D) -> io::Result<()> {
        let data = utils::serialize_to_vec(value)?;
        self.write_frame(data).await
    }

    /// Executes the async function while the [`Backup`] of this transport is frozen.
    pub async fn do_frozen<F, X>(&mut self, mut f: F) -> io::Result<()>
    where
        F: FnMut(&mut Self) -> X,
        X: Future<Output = io::Result<()>>,
    {
        let is_frozen = self.backup.is_frozen();
        self.backup.freeze();
        let result = f(self).await;
        self.backup.set_frozen(is_frozen);
        result
    }

    /// Places the transport in **synchronize mode** where it communicates with the other side how
    /// many frames have been sent and received. From there, any frames not received by the other
    /// side are sent again and then this transport waits for any missing frames that it did not
    /// receive from the other side.
    ///
    /// ### Note
    ///
    /// This will clear the internal incoming and outgoing buffers, so any frame that was in
    /// transit in either direction will be dropped.
    pub async fn synchronize(&mut self) -> io::Result<()> {
        async fn synchronize_impl<T: Transport>(
            this: &mut FramedTransport<T>,
            backup: &mut Backup,
        ) -> io::Result<()> {
            type Stats = (u64, u64, u64);

            // Stats in the form of (sent, received, available)
            let sent_cnt: u64 = backup.sent_cnt();
            let received_cnt: u64 = backup.received_cnt();
            let available_cnt: u64 = backup
                .frame_cnt()
                .try_into()
                .expect("Cannot case usize to u64");

            // Clear our internal buffers
            this.clear();

            // Communicate frame counters with other side so we can determine how many frames to send
            // and how many to receive. Wait until we get the stats from the other side, and then send
            // over any missing frames.
            trace!(
                "Stats: sent = {sent_cnt}, received = {received_cnt}, available = {available_cnt}"
            );
            this.write_frame_for(&(sent_cnt, received_cnt, available_cnt))
                .await?;
            let (other_sent_cnt, other_received_cnt, other_available_cnt) =
                this.read_frame_as::<Stats>().await?.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Transport terminated before getting replay stats",
                    )
                })?;
            trace!("Other stats: sent = {other_sent_cnt}, received = {other_received_cnt}, available = {other_available_cnt}");

            // Determine how many frames we need to resend. This will either be (sent - received) or
            // available frames, whichever is smaller.
            let resend_cnt = std::cmp::min(
                if sent_cnt > other_received_cnt {
                    sent_cnt - other_received_cnt
                } else {
                    0
                },
                available_cnt,
            );

            // Determine how many frames we expect to receive. This will either be (received - sent) or
            // available frames, whichever is smaller.
            let expected_cnt = std::cmp::min(
                if received_cnt < other_sent_cnt {
                    other_sent_cnt - received_cnt
                } else {
                    0
                },
                other_available_cnt,
            );

            // Send all missing frames, removing any frames that we know have been received
            trace!("Reducing internal replay frames to {resend_cnt}");
            backup.truncate_front(resend_cnt.try_into().expect("Cannot cast usize to u64"));

            debug!("Sending {resend_cnt} frames");
            for frame in backup.frames() {
                this.try_write_frame(frame.as_borrowed())?;
            }
            this.flush().await?;

            // Receive all expected frames, placing their contents into our incoming queue
            //
            // NOTE: We do not increment our counter as this is done during `try_read_frame`, even
            //       when the frame comes from our internal queue. To avoid duplicating the increment,
            //       we do not increment the counter here.
            debug!("Waiting for {expected_cnt} frames");
            for i in 0..expected_cnt {
                let frame = this.read_frame().await?.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        format!(
                            "Transport terminated before getting frame {}/{expected_cnt}",
                            i + 1
                        ),
                    )
                })?;

                // Encode our frame and write it to be queued in our incoming data
                // NOTE: We have to do encoding here as incoming bytes are expected to be encoded
                this.codec.encode(frame)?.write(&mut this.incoming);
            }

            // Catch up our read count as we can have the case where the other side has a higher
            // count than frames sent if some frames were fully dropped due to size limits
            if backup.received_cnt() != other_sent_cnt {
                warn!(
                    "Backup received count ({}) != other sent count ({}), so resetting to match",
                    backup.received_cnt(),
                    other_sent_cnt
                );
                backup.set_received_cnt(other_sent_cnt);
            }

            Ok(())
        }

        // Swap out our backup so we don't mutate it from synchronization efforts
        let mut backup = std::mem::take(&mut self.backup);

        // Perform our operation, but don't return immediately so we can restore our backup
        let result = synchronize_impl(self, &mut backup).await;

        // Reset our backup to the real version
        self.backup = backup;

        result
    }

    /// Shorthand for creating a [`FramedTransport`] with a [`PlainCodec`] and then immediately
    /// performing a [`client_handshake`], returning the updated [`FramedTransport`] on success.
    ///
    /// [`client_handshake`]: FramedTransport::client_handshake
    #[inline]
    pub async fn from_client_handshake(transport: T) -> io::Result<Self> {
        let mut transport = Self::plain(transport);
        transport.client_handshake().await?;
        Ok(transport)
    }

    /// Perform the client-side of a handshake. See [`handshake`] for more details.
    ///
    /// [`handshake`]: FramedTransport::handshake
    pub async fn client_handshake(&mut self) -> io::Result<()> {
        self.handshake(Handshake::client()).await
    }

    /// Shorthand for creating a [`FramedTransport`] with a [`PlainCodec`] and then immediately
    /// performing a [`server_handshake`], returning the updated [`FramedTransport`] on success.
    ///
    /// [`client_handshake`]: FramedTransport::client_handshake
    #[inline]
    pub async fn from_server_handshake(transport: T) -> io::Result<Self> {
        let mut transport = Self::plain(transport);
        transport.server_handshake().await?;
        Ok(transport)
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

        // Swap out our backup so we don't mutate it from synchronization efforts
        let backup = std::mem::take(&mut self.backup);

        // Transform the transport's codec to abide by the choice. In the case of an error, we
        // reset the codec back to what it was prior to attempting the handshake and clear the
        // internal buffers as they may be corrupt.
        match self.handshake_impl(handshake).await {
            Ok(codec) => {
                self.set_codec(codec);
                self.backup = backup;
                Ok(())
            }
            Err(x) => {
                self.set_codec(old_codec);
                self.clear();
                self.backup = backup;
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

        // Define a label to distinguish log output for client and server
        let log_label = if handshake.is_client() {
            "Handshake | Client"
        } else {
            "Handshake | Server"
        };

        // Determine compression and encryption to apply to framed transport
        let choice = match handshake {
            Handshake::Client {
                preferred_compression_type,
                preferred_compression_level,
                preferred_encryption_type,
            } => {
                // Receive options from the server and pick one
                debug!("[{log_label}] Waiting on options");
                let options = self.read_frame_as::<Options>().await?.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Transport closed early while waiting for options",
                    )
                })?;

                // Choose a compression and encryption option from the options
                debug!("[{log_label}] Selecting from options: {options:?}");
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
                debug!("[{log_label}] Reporting choice: {choice:?}");
                self.write_frame_for(&choice).await?;

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
                debug!("[{log_label}] Sending options: {options:?}");
                self.write_frame_for(&options).await?;

                // Get client's response with selected compression and encryption
                debug!("[{log_label}] Waiting on choice");
                self.read_frame_as::<Choice>().await?.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Transport closed early while waiting for choice",
                    )
                })?
            }
        };

        debug!("[{log_label}] Building compression & encryption codecs based on {choice:?}");
        let compression_level = choice.compression_level.unwrap_or_default();

        // Acquire a codec for the compression type
        let compression_codec = choice
            .compression_type
            .map(|ty| ty.new_codec(compression_level))
            .transpose()?;

        // In the case that we are using encryption, we derive a shared secret key to use with the
        // encryption type
        let encryption_codec = match choice.encryption_type {
            // Fail early if we got an unknown encryption type
            Some(EncryptionType::Unknown) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unknown compression type",
                ))
            }
            Some(ty) => {
                let key = self.exchange_keys_impl(log_label).await?;
                Some(ty.new_codec(key.unprotected_as_bytes())?)
            }
            None => None,
        };

        // Bundle our compression and encryption codecs into a single, chained codec
        trace!("[{log_label}] Bundling codecs");
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

    /// Places the transport into key-exchange mode where it attempts to derive a shared secret key
    /// with the other transport.
    pub async fn exchange_keys(&mut self) -> io::Result<SecretKey32> {
        self.exchange_keys_impl("").await
    }

    async fn exchange_keys_impl(&mut self, label: &str) -> io::Result<SecretKey32> {
        let log_label = if label.is_empty() {
            String::new()
        } else {
            format!("[{label}] ")
        };

        #[derive(Serialize, Deserialize)]
        struct KeyExchangeData {
            /// Bytes of the public key
            #[serde(with = "serde_bytes")]
            public_key: PublicKeyBytes,

            /// Randomly generated salt
            #[serde(with = "serde_bytes")]
            salt: Salt,
        }

        debug!("{log_label}Exchanging public key and salt");
        let exchange = KeyExchange::default();
        self.write_frame_for(&KeyExchangeData {
            public_key: exchange.pk_bytes(),
            salt: *exchange.salt(),
        })
        .await?;

        // TODO: This key only works because it happens to be 32 bytes and our encryption
        //       also wants a 32-byte key. Once we introduce new encryption algorithms that
        //       are not using 32-byte keys, the key exchange will need to support deriving
        //       other length keys.
        trace!("{log_label}Waiting on public key and salt from other side");
        let data = self
            .read_frame_as::<KeyExchangeData>()
            .await?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Transport closed early while waiting for key data",
                )
            })?;

        trace!("{log_label}Deriving shared secret key");
        let key = exchange.derive_shared_secret(data.public_key, data.salt)?;
        Ok(key)
    }
}

#[async_trait]
impl<T> Reconnectable for FramedTransport<T>
where
    T: Transport,
{
    async fn reconnect(&mut self) -> io::Result<()> {
        Reconnectable::reconnect(&mut self.inner).await
    }
}

impl FramedTransport<InmemoryTransport> {
    /// Produces a pair of inmemory transports that are connected to each other using a
    /// [`PlainCodec`].
    ///
    /// Sets the buffer for message passing for each underlying transport to the given buffer size.
    pub fn pair(
        buffer: usize,
    ) -> (
        FramedTransport<InmemoryTransport>,
        FramedTransport<InmemoryTransport>,
    ) {
        let (a, b) = InmemoryTransport::pair(buffer);
        let a = FramedTransport::new(a, Box::new(PlainCodec::new()));
        let b = FramedTransport::new(b, Box::new(PlainCodec::new()));
        (a, b)
    }

    /// Links the underlying transports together using [`InmemoryTransport::link`].
    pub fn link(&mut self, other: &mut Self, buffer: usize) {
        self.inner.link(&mut other.inner, buffer)
    }
}

#[cfg(test)]
impl FramedTransport<InmemoryTransport> {
    /// Generates a test pair with default capacity
    pub fn test_pair(
        buffer: usize,
    ) -> (
        FramedTransport<InmemoryTransport>,
        FramedTransport<InmemoryTransport>,
    ) {
        Self::pair(buffer)
    }
}

#[cfg(test)]
mod tests {
    use bytes::BufMut;
    use test_log::test;

    use super::*;
    use crate::common::TestTransport;

    /// Codec that always succeeds without altering the frame
    #[derive(Clone, Debug, PartialEq, Eq)]
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
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct ErrCodec;

    impl Codec for ErrCodec {
        fn encode<'a>(&mut self, _frame: Frame<'a>) -> io::Result<Frame<'a>> {
            Err(io::Error::from(io::ErrorKind::Other))
        }

        fn decode<'a>(&mut self, _frame: Frame<'a>) -> io::Result<Frame<'a>> {
            Err(io::Error::from(io::ErrorKind::Other))
        }
    }

    // Hardcoded custom codec so we can verify it works differently than plain codec
    #[derive(Clone)]
    struct CustomCodec;

    impl Codec for CustomCodec {
        fn encode<'a>(&mut self, _: Frame<'a>) -> io::Result<Frame<'a>> {
            Ok(Frame::new(b"encode"))
        }

        fn decode<'a>(&mut self, _: Frame<'a>) -> io::Result<Frame<'a>> {
            Ok(Frame::new(b"decode"))
        }
    }

    type SimulateTryReadFn = Box<dyn Fn(&mut [u8]) -> io::Result<usize> + Send + Sync>;

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
    ) -> SimulateTryReadFn {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Stuff all of our frames into a single byte collection
        let data = {
            let mut buf = BytesMut::new();

            for frame in frames {
                frame.write(&mut buf);
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
            Box::new(OkCodec),
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
            Box::new(OkCodec),
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
            Box::new(OkCodec),
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
            Frame::new(b"hello world").write(&mut data);
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
            Box::new(OkCodec),
        );
        assert_eq!(transport.try_read_frame().unwrap().unwrap(), b"hello world");
    }

    #[test]
    fn try_read_frame_should_return_next_available_frame_if_already_in_incoming_buffer() {
        // Store two frames in our data to transmit
        let data = {
            let mut data = BytesMut::new();
            Frame::new(b"hello world").write(&mut data);
            Frame::new(b"hello again").write(&mut data);
            data.freeze()
        };

        // Configure transport to return both frames in single read such that we have another
        // complete frame to parse (in the case that an underlying try_read would block, but we had
        // data available before that)
        let mut transport = FramedTransport::new(
            TestTransport {
                f_try_read: Box::new(move |buf| {
                    static mut CNT: usize = 0;
                    unsafe {
                        CNT += 1;
                        if CNT == 2 {
                            Err(io::Error::from(io::ErrorKind::WouldBlock))
                        } else {
                            let n = data.len();
                            buf[..data.len()].copy_from_slice(data.as_ref());
                            Ok(n)
                        }
                    }
                }),
                f_ready: Box::new(|_| Ok(Ready::READABLE)),
                ..Default::default()
            },
            Box::new(OkCodec),
        );

        // Read first frame
        assert_eq!(transport.try_read_frame().unwrap().unwrap(), b"hello world");

        // Read second frame
        assert_eq!(transport.try_read_frame().unwrap().unwrap(), b"hello again");
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
        let mut transport = FramedTransport::new(
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
        let mut transport = FramedTransport::new(
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
        let mut transport = FramedTransport::new(
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
        let mut transport = FramedTransport::new(
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
        let mut transport = FramedTransport::new(
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
        let mut transport = FramedTransport::new(
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

    #[inline]
    async fn test_synchronize_stats(
        transport: &mut FramedTransport<InmemoryTransport>,
        sent_cnt: u64,
        received_cnt: u64,
        available_cnt: u64,
        expected_sent_cnt: u64,
        expected_received_cnt: u64,
        expected_available_cnt: u64,
    ) {
        // From the other side, claim that we have received 2 frames
        // (sent, received, available)
        transport
            .write_frame_for(&(sent_cnt, received_cnt, available_cnt))
            .await
            .unwrap();

        // Receive stats from the other side
        let (sent, received, available) = transport
            .read_frame_as::<(u64, u64, u64)>()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sent, expected_sent_cnt, "Wrong sent cnt");
        assert_eq!(received, expected_received_cnt, "Wrong received cnt");
        assert_eq!(available, expected_available_cnt, "Wrong available cnt");
    }

    #[test(tokio::test)]
    async fn synchronize_should_resend_no_frames_if_other_side_claims_it_has_more_than_us() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Configure the backup such that we have sent one frame
        t2.backup.push_frame(Frame::new(b"hello world"));
        t2.backup.increment_sent_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let _task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 0, 2, 0
        // expected (sent, received, available) = 1, 0, 1
        test_synchronize_stats(&mut t1, 0, 2, 0, 1, 0, 1).await;

        // Should not receive anything before our done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");
    }

    #[test(tokio::test)]
    async fn synchronize_should_resend_no_frames_if_none_missing_on_other_side() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Configure the backup such that we have sent one frame
        t2.backup.push_frame(Frame::new(b"hello world"));
        t2.backup.increment_sent_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let _task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 0, 1, 0
        // expected (sent, received, available) = 1, 0, 1
        test_synchronize_stats(&mut t1, 0, 1, 0, 1, 0, 1).await;

        // Should not receive anything before our done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");
    }

    #[test(tokio::test)]
    async fn synchronize_should_resend_some_frames_if_some_missing_on_other_side() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Configure the backup such that we have sent two frames
        t2.backup.push_frame(Frame::new(b"hello"));
        t2.backup.push_frame(Frame::new(b"world"));
        t2.backup.increment_sent_cnt();
        t2.backup.increment_sent_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let _task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 0, 1, 0
        // expected (sent, received, available) = 2, 0, 2
        test_synchronize_stats(&mut t1, 0, 1, 0, 2, 0, 2).await;

        // Recieve both frames and then the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"world");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");
    }

    #[test(tokio::test)]
    async fn synchronize_should_resend_all_frames_if_all_missing_on_other_side() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Configure the backup such that we have sent two frames
        t2.backup.push_frame(Frame::new(b"hello"));
        t2.backup.push_frame(Frame::new(b"world"));
        t2.backup.increment_sent_cnt();
        t2.backup.increment_sent_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let _task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 0, 0, 0
        // expected (sent, received, available) = 2, 0, 2
        test_synchronize_stats(&mut t1, 0, 0, 0, 2, 0, 2).await;

        // Recieve both frames and then the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"hello");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"world");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");
    }

    #[test(tokio::test)]
    async fn synchronize_should_resend_available_frames_if_more_than_available_missing_on_other_side(
    ) {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Configure the backup such that we have sent two frames, and believe that we have
        // sent 3 in total, a situation that happens once we reach the peak possible size of
        // old frames to store
        t2.backup.push_frame(Frame::new(b"hello"));
        t2.backup.push_frame(Frame::new(b"world"));
        t2.backup.increment_sent_cnt();
        t2.backup.increment_sent_cnt();
        t2.backup.increment_sent_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let _task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 0, 0, 0
        // expected (sent, received, available) = 3, 0, 2
        test_synchronize_stats(&mut t1, 0, 0, 0, 3, 0, 2).await;

        // Recieve both frames and then the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"hello");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"world");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");
    }

    #[test(tokio::test)]
    async fn synchronize_should_receive_no_frames_if_other_side_claims_it_has_more_than_us() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Mark other side as having received a frame
        t2.backup.increment_received_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let _task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 0, 0, 0
        // expected (sent, received, available) = 0, 1, 0
        test_synchronize_stats(&mut t1, 0, 0, 0, 0, 1, 0).await;

        // Recieve the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");
    }

    #[test(tokio::test)]
    async fn synchronize_should_receive_no_frames_if_none_missing_from_other_side() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Mark other side as having received a frame
        t2.backup.increment_received_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let _task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 1, 0, 1
        // expected (sent, received, available) = 0, 1, 0
        test_synchronize_stats(&mut t1, 1, 0, 1, 0, 1, 0).await;

        // Recieve the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");
    }

    #[test(tokio::test)]
    async fn synchronize_should_receive_some_frames_if_some_missing_from_other_side() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Mark other side as having received a frame
        t2.backup.increment_received_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 2, 0, 2
        // expected (sent, received, available) = 0, 1, 0
        test_synchronize_stats(&mut t1, 2, 0, 2, 0, 1, 0).await;

        // Send a frame to fill the gap
        t1.write_frame(Frame::new(b"hello")).await.unwrap();

        // Recieve the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");

        // Drop the transport such that the other side will get a definite termination
        drop(t1);

        // Verify that the frame was captured on the other side
        let mut t2 = task.await.unwrap();
        assert_eq!(t2.read_frame().await.unwrap().unwrap(), b"hello");
        assert_eq!(t2.read_frame().await.unwrap(), None);
    }

    #[test(tokio::test)]
    async fn synchronize_should_receive_all_frames_if_all_missing_from_other_side() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 2, 0, 2
        // expected (sent, received, available) = 0, 0, 0
        test_synchronize_stats(&mut t1, 2, 0, 2, 0, 0, 0).await;

        // Send frames to fill the gap
        t1.write_frame(Frame::new(b"hello")).await.unwrap();
        t1.write_frame(Frame::new(b"world")).await.unwrap();

        // Recieve the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");

        // Drop the transport such that the other side will get a definite termination
        drop(t1);

        // Verify that the frame was captured on the other side
        let mut t2 = task.await.unwrap();
        assert_eq!(t2.read_frame().await.unwrap().unwrap(), b"hello");
        assert_eq!(t2.read_frame().await.unwrap().unwrap(), b"world");
        assert_eq!(t2.read_frame().await.unwrap(), None);
    }

    #[test(tokio::test)]
    async fn synchronize_should_receive_all_frames_if_more_than_all_missing_from_other_side() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 3, 0, 2
        // expected (sent, received, available) = 0, 0, 0
        test_synchronize_stats(&mut t1, 2, 0, 2, 0, 0, 0).await;

        // Send frames to fill the gap
        t1.write_frame(Frame::new(b"hello")).await.unwrap();
        t1.write_frame(Frame::new(b"world")).await.unwrap();

        // Recieve the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");

        // Drop the transport such that the other side will get a definite termination
        drop(t1);

        // Verify that the frame was captured on the other side
        let mut t2 = task.await.unwrap();
        assert_eq!(t2.read_frame().await.unwrap().unwrap(), b"hello");
        assert_eq!(t2.read_frame().await.unwrap().unwrap(), b"world");
        assert_eq!(t2.read_frame().await.unwrap(), None);
    }

    #[test(tokio::test)]
    async fn synchronize_should_fail_if_connection_terminated_before_receiving_missing_frames() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 2, 0, 2
        // expected (sent, received, available) = 0, 0, 0
        test_synchronize_stats(&mut t1, 2, 0, 2, 0, 0, 0).await;

        // Send one frame to fill the gap
        t1.write_frame(Frame::new(b"hello")).await.unwrap();

        // Drop the transport to cause a failure
        drop(t1);

        // Verify that the other side's synchronization failed
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn synchronize_should_fail_if_connection_terminated_while_waiting_for_frame_stats() {
        let (t1, mut t2) = FramedTransport::pair(100);

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // Drop the transport to cause a failure
        drop(t1);

        // Verify that the other side's synchronization failed
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn synchronize_should_clear_any_prexisting_incoming_and_outgoing_data() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Put some frames into the incoming and outgoing of our transport
        Frame::new(b"bad incoming").write(&mut t2.incoming);
        Frame::new(b"bad outgoing").write(&mut t2.outgoing);

        // Configure the backup such that we have sent two frames
        t2.backup.push_frame(Frame::new(b"hello"));
        t2.backup.push_frame(Frame::new(b"world"));
        t2.backup.increment_sent_cnt();
        t2.backup.increment_sent_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2
        });

        // fake     (sent, received, available) = 2, 0, 2
        // expected (sent, received, available) = 2, 0, 2
        test_synchronize_stats(&mut t1, 2, 0, 2, 2, 0, 2).await;

        // Send frames to fill the gap
        t1.write_frame(Frame::new(b"one")).await.unwrap();
        t1.write_frame(Frame::new(b"two")).await.unwrap();

        // Recieve both frames and then the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"hello");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"world");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");

        // Drop the transport such that the other side will get a definite termination
        drop(t1);

        // Verify that the frame was captured on the other side
        let mut t2 = task.await.unwrap();
        assert_eq!(t2.read_frame().await.unwrap().unwrap(), b"one");
        assert_eq!(t2.read_frame().await.unwrap().unwrap(), b"two");
        assert_eq!(t2.read_frame().await.unwrap(), None);
    }

    #[test(tokio::test)]
    async fn synchronize_should_not_increment_the_sent_frames_or_store_replayed_frames_in_the_backup(
    ) {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Configure the backup such that we have sent two frames
        t2.backup.push_frame(Frame::new(b"hello"));
        t2.backup.push_frame(Frame::new(b"world"));
        t2.backup.increment_sent_cnt();
        t2.backup.increment_sent_cnt();

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();

            t2.backup.freeze();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2.backup.unfreeze();

            t2
        });

        // fake     (sent, received, available) = 0, 0, 0
        // expected (sent, received, available) = 2, 0, 2
        test_synchronize_stats(&mut t1, 0, 0, 0, 2, 0, 2).await;

        // Recieve both frames and then the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"hello");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"world");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");

        // Drop the transport such that the other side will get a definite termination
        drop(t1);

        // Verify that the backup on the other side was unaltered by the frames being sent
        let t2 = task.await.unwrap();
        assert_eq!(t2.backup.sent_cnt(), 2, "Wrong sent cnt");
        assert_eq!(t2.backup.received_cnt(), 0, "Wrong received cnt");
        assert_eq!(t2.backup.frame_cnt(), 2, "Wrong frame cnt");
    }

    #[test(tokio::test)]
    async fn synchronize_should_update_the_backup_received_cnt_to_match_other_side_sent() {
        let (mut t1, mut t2) = FramedTransport::pair(100);

        // Spawn a separate task to do synchronization simulation so we don't deadlock, and also
        // send a frame to indicate when finished so we can know when synchronization is done
        // during our test
        let task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();

            t2.backup.freeze();
            t2.write_frame(Frame::new(b"done")).await.unwrap();
            t2.backup.unfreeze();

            t2
        });

        // fake     (sent, received, available) = 2, 0, 1
        // expected (sent, received, available) = 0, 0, 0
        test_synchronize_stats(&mut t1, 2, 0, 1, 0, 0, 0).await;

        // Send frames to fill the gap
        t1.write_frame(Frame::new(b"hello")).await.unwrap();

        // Recieve both frames and then the done indicator
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"done");

        // Drop the transport such that the other side will get a definite termination
        drop(t1);

        // Verify that the backup on the other side updated based on sent count and not available
        let t2 = task.await.unwrap();
        assert_eq!(t2.backup.sent_cnt(), 0, "Wrong sent cnt");
        assert_eq!(t2.backup.received_cnt(), 2, "Wrong received cnt");
        assert_eq!(t2.backup.frame_cnt(), 0, "Wrong frame cnt");
    }

    #[test(tokio::test)]
    async fn synchronize_should_work_even_if_codec_changes_between_attempts() {
        let (mut t1, _t1_other) = FramedTransport::pair(100);
        let (mut t2, _t2_other) = FramedTransport::pair(100);

        // Send some frames from each side
        t1.write_frame(Frame::new(b"hello")).await.unwrap();
        t1.write_frame(Frame::new(b"world")).await.unwrap();
        t2.write_frame(Frame::new(b"foo")).await.unwrap();
        t2.write_frame(Frame::new(b"bar")).await.unwrap();

        // Drop the other transports, link our real transports together, and change the codec
        drop(_t1_other);
        drop(_t2_other);
        t1.link(&mut t2, 100);
        let codec = EncryptionCodec::new_xchacha20poly1305(Default::default());
        t1.codec = Box::new(codec.clone());
        t2.codec = Box::new(codec);

        // Spawn a separate task to do synchronization so we don't deadlock
        let task = tokio::spawn(async move {
            t2.synchronize().await.unwrap();
            t2
        });

        t1.synchronize().await.unwrap();

        // Verify that we get the appropriate frames from both sides
        let mut t2 = task.await.unwrap();
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"foo");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"bar");
        assert_eq!(t2.read_frame().await.unwrap().unwrap(), b"hello");
        assert_eq!(t2.read_frame().await.unwrap().unwrap(), b"world");
    }

    #[test(tokio::test)]
    async fn handshake_should_configure_transports_with_matching_codec() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // NOTE: Spawn a separate task for one of our transports so we can communicate without
        //       deadlocking
        let task = tokio::spawn(async move {
            // Wait for handshake to complete
            t2.server_handshake().await.unwrap();

            // Receive one frame and echo it back
            let frame = t2.read_frame().await.unwrap().unwrap();
            t2.write_frame(frame).await.unwrap();
        });

        t1.client_handshake().await.unwrap();

        // Verify that the transports can still communicate with one another
        t1.write_frame(b"hello world").await.unwrap();
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"hello world");

        // Ensure that the other transport did not error
        task.await.unwrap();
    }

    #[test(tokio::test)]
    async fn handshake_failing_should_ensure_existing_codec_remains() {
        let (mut t1, t2) = FramedTransport::test_pair(100);

        // Set a different codec on our transport so we can verify it doesn't change
        t1.set_codec(Box::new(CustomCodec));

        // Drop our transport on the other side to cause an immediate failure
        drop(t2);

        // Ensure we detect the failure on handshake
        t1.client_handshake().await.unwrap_err();

        // Verify that the codec did not reset to plain text by using the codec
        assert_eq!(t1.codec.encode(Frame::new(b"test")).unwrap(), b"encode");
        assert_eq!(t1.codec.decode(Frame::new(b"test")).unwrap(), b"decode");
    }

    #[test(tokio::test)]
    async fn handshake_should_clear_any_intermittent_buffer_contents_prior_to_handshake_failing() {
        let (mut t1, t2) = FramedTransport::test_pair(100);

        // Set a different codec on our transport so we can verify it doesn't change
        t1.set_codec(Box::new(CustomCodec));

        // Drop our transport on the other side to cause an immediate failure
        drop(t2);

        // Put some garbage in our buffers
        t1.incoming.extend_from_slice(b"garbage in");
        t1.outgoing.extend_from_slice(b"garbage out");

        // Ensure we detect the failure on handshake
        t1.client_handshake().await.unwrap_err();

        // Verify that the incoming and outgoing buffers are empty
        assert!(t1.incoming.is_empty());
        assert!(t1.outgoing.is_empty());
    }

    #[test(tokio::test)]
    async fn handshake_should_clear_any_intermittent_buffer_contents_prior_to_handshake_succeeding()
    {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // NOTE: Spawn a separate task for one of our transports so we can communicate without
        //       deadlocking
        let task = tokio::spawn(async move {
            // Wait for handshake to complete
            t2.server_handshake().await.unwrap();

            // Receive one frame and echo it back
            let frame = t2.read_frame().await.unwrap().unwrap();
            t2.write_frame(frame).await.unwrap();
        });

        // Put some garbage in our buffers
        t1.incoming.extend_from_slice(b"garbage in");
        t1.outgoing.extend_from_slice(b"garbage out");

        t1.client_handshake().await.unwrap();

        // Verify that the transports can still communicate with one another
        t1.write_frame(b"hello world").await.unwrap();
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"hello world");

        // Ensure that the other transport did not error
        task.await.unwrap();

        // Verify that the incoming and outgoing buffers are empty
        assert!(t1.incoming.is_empty());
        assert!(t1.outgoing.is_empty());
    }

    #[test(tokio::test)]
    async fn handshake_for_client_should_fail_if_receives_unexpected_frame_instead_of_options() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // NOTE: Spawn a separate task for one of our transports so we can communicate without
        //       deadlocking
        let task = tokio::spawn(async move {
            t2.write_frame(b"not a valid frame for handshake")
                .await
                .unwrap();
        });

        // Ensure we detect the failure on handshake
        let err = t1.client_handshake().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // Ensure that the other transport did not error
        task.await.unwrap();
    }

    #[test(tokio::test)]
    async fn handshake_for_client_should_fail_unable_to_send_codec_choice_to_other_side() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        #[derive(Debug, Serialize, Deserialize)]
        struct Options {
            compression_types: Vec<CompressionType>,
            encryption_types: Vec<EncryptionType>,
        }

        // NOTE: Spawn a separate task for one of our transports so we can communicate without
        //       deadlocking
        let task = tokio::spawn(async move {
            // Send options, and then quit so the client side will fail
            t2.write_frame_for(&Options {
                compression_types: Vec::new(),
                encryption_types: Vec::new(),
            })
            .await
            .unwrap();
        });

        // Ensure we detect the failure on handshake
        let err = t1.client_handshake().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WriteZero);

        // Ensure that the other transport did not error
        task.await.unwrap();
    }

    #[test(tokio::test)]
    async fn handshake_for_client_should_fail_if_unable_to_receive_key_exchange_data_from_other_side(
    ) {
        #[derive(Debug, Serialize, Deserialize)]
        struct Options {
            compression_types: Vec<CompressionType>,
            encryption_types: Vec<EncryptionType>,
        }

        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Go ahead and queue up a choice, and then queue up invalid key exchange data
        t2.write_frame_for(&Options {
            compression_types: CompressionType::known_variants().to_vec(),
            encryption_types: EncryptionType::known_variants().to_vec(),
        })
        .await
        .unwrap();

        t2.write_frame(b"not valid key exchange data")
            .await
            .unwrap();

        // Ensure we detect the failure on handshake
        let err = t1.client_handshake().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test(tokio::test)]
    async fn handshake_for_server_should_fail_if_receives_unexpected_frame_instead_of_choice() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // NOTE: Spawn a separate task for one of our transports so we can communicate without
        //       deadlocking
        let task = tokio::spawn(async move {
            t2.write_frame(b"not a valid frame for handshake")
                .await
                .unwrap();
        });

        // Ensure we detect the failure on handshake
        let err = t1.server_handshake().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);

        // Ensure that the other transport did not error
        task.await.unwrap();
    }

    #[test(tokio::test)]
    async fn handshake_for_server_should_fail_unable_to_send_codec_options_to_other_side() {
        let (mut t1, t2) = FramedTransport::test_pair(100);

        // Drop our other transport to ensure that nothing can be sent to it
        drop(t2);

        // Ensure we detect the failure on handshake
        let err = t1.server_handshake().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WriteZero);
    }

    #[test(tokio::test)]
    async fn handshake_for_server_should_fail_if_selected_codec_choice_uses_an_unknown_compression_type(
    ) {
        #[derive(Debug, Serialize, Deserialize)]
        struct Choice {
            compression_level: Option<CompressionLevel>,
            compression_type: Option<CompressionType>,
            encryption_type: Option<EncryptionType>,
        }

        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Go ahead and queue up an improper response
        t2.write_frame_for(&Choice {
            compression_level: None,
            compression_type: Some(CompressionType::Unknown),
            encryption_type: None,
        })
        .await
        .unwrap();

        // Ensure we detect the failure on handshake
        let err = t1.server_handshake().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test(tokio::test)]
    async fn handshake_for_server_should_fail_if_selected_codec_choice_uses_an_unknown_encryption_type(
    ) {
        #[derive(Debug, Serialize, Deserialize)]
        struct Choice {
            compression_level: Option<CompressionLevel>,
            compression_type: Option<CompressionType>,
            encryption_type: Option<EncryptionType>,
        }

        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Go ahead and queue up an improper response
        t2.write_frame_for(&Choice {
            compression_level: None,
            compression_type: None,
            encryption_type: Some(EncryptionType::Unknown),
        })
        .await
        .unwrap();

        // Ensure we detect the failure on handshake
        let err = t1.server_handshake().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test(tokio::test)]
    async fn handshake_for_server_should_fail_if_unable_to_receive_key_exchange_data_from_other_side(
    ) {
        #[derive(Debug, Serialize, Deserialize)]
        struct Choice {
            compression_level: Option<CompressionLevel>,
            compression_type: Option<CompressionType>,
            encryption_type: Option<EncryptionType>,
        }

        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Go ahead and queue up a choice, and then queue up invalid key exchange data
        t2.write_frame_for(&Choice {
            compression_level: None,
            compression_type: None,
            encryption_type: Some(EncryptionType::XChaCha20Poly1305),
        })
        .await
        .unwrap();

        t2.write_frame(b"not valid key exchange data")
            .await
            .unwrap();

        // Ensure we detect the failure on handshake
        let err = t1.server_handshake().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test(tokio::test)]
    async fn exchange_keys_should_fail_if_unable_to_send_exchange_data_to_other_side() {
        let (mut t1, t2) = FramedTransport::test_pair(100);

        // Drop the other side to ensure that the exchange fails at the beginning
        drop(t2);

        // Perform key exchange and verify error is as expected
        assert_eq!(
            t1.exchange_keys().await.unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
    }

    #[test(tokio::test)]
    async fn exchange_keys_should_fail_if_received_invalid_exchange_data() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up an invalid exchange response
        t2.write_frame(b"some invalid frame").await.unwrap();

        // Perform key exchange and verify error is as expected
        assert_eq!(
            t1.exchange_keys().await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test(tokio::test)]
    async fn exchange_keys_should_return_shared_secret_key_if_successful() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Spawn a task to avoid deadlocking
        let task = tokio::spawn(async move { t2.exchange_keys().await.unwrap() });

        // Perform key exchange
        let key = t1.exchange_keys().await.unwrap();

        // Validate that the keys on both sides match
        assert_eq!(key, task.await.unwrap());
    }
}
