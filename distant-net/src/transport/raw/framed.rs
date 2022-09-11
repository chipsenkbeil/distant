use super::{Interest, RawTransport, Ready, Reconnectable};
use async_trait::async_trait;
use std::io;

mod codec;
pub use codec::*;

/// Represents a [`RawTransport`] that reads and writes using frames defined by a [`Codec`],
/// which provides the ability to guarantee that data is read and written completely and also
/// follows the format of the given codec such as encryption and authentication of bytes
pub struct FramedRawTransport<T, C>
where
    T: RawTransport,
    C: Codec,
{
    inner: T,
    codec: C,
}

#[async_trait]
impl<T, C> Reconnectable for FramedRawTransport<T, C>
where
    T: RawTransport,
    C: Codec,
{
    async fn reconnect(&mut self) -> io::Result<()> {
        Reconnectable::reconnect(&mut self.inner).await
    }
}

#[async_trait]
impl<T, C> RawTransport for FramedRawTransport<T, C>
where
    T: RawTransport,
    C: Codec,
{
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        todo!();
    }

    fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        todo!();
    }

    async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        todo!();
    }
}

impl FramedRawTransport<super::InmemoryTransport, PlainCodec> {
    /// Produces a pair of inmemory transports that are connected to each other using
    /// a standard codec
    ///
    /// Sets the buffer for message passing for each underlying transport to the given buffer size
    pub fn pair(
        buffer: usize,
    ) -> (
        FramedRawTransport<super::InmemoryTransport, PlainCodec>,
        FramedRawTransport<super::InmemoryTransport, PlainCodec>,
    ) {
        let (a, b) = super::InmemoryTransport::pair(buffer);
        let a = FramedRawTransport::new(a, PlainCodec::new());
        let b = FramedRawTransport::new(b, PlainCodec::new());
        (a, b)
    }
}
