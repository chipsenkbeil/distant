use crate::{
    utils, Codec, IntoSplit, RawTransport, UntypedTransport, UntypedTransportRead,
    UntypedTransportWrite,
};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use std::io;
use tokio_util::codec::{Framed, FramedRead, FramedWrite};

#[cfg(test)]
mod test;

#[cfg(test)]
pub use test::*;

mod read;
pub use read::*;

mod write;
pub use write::*;

/// Represents [`TypedTransport`] of data across the network using frames in order to support
/// typed messages instead of arbitrary bytes being sent across the wire.
///
/// Note that this type does **not** implement [`RawTransport`] and instead acts as a wrapper
/// around a transport to provide a higher-level interface
#[derive(Debug)]
pub struct FramedTransport<T, C>(Framed<T, C>)
where
    T: RawTransport,
    C: Codec;

impl<T, C> FramedTransport<T, C>
where
    T: RawTransport,
    C: Codec,
{
    /// Creates a new instance of the transport, wrapping the stream in a `Framed<T, XChaCha20Poly1305Codec>`
    pub fn new(transport: T, codec: C) -> Self {
        Self(Framed::new(transport, codec))
    }
}

impl<T, C> UntypedTransport for FramedTransport<T, C>
where
    T: RawTransport,
    C: Codec + Send,
{
    type ReadHalf = FramedTransportReadHalf<T::ReadHalf, C>;
    type WriteHalf = FramedTransportWriteHalf<T::WriteHalf, C>;
}

impl<T, C> IntoSplit for FramedTransport<T, C>
where
    T: RawTransport,
    C: Codec,
{
    type Read = FramedTransportReadHalf<T::ReadHalf, C>;
    type Write = FramedTransportWriteHalf<T::WriteHalf, C>;

    fn into_split(self) -> (Self::Write, Self::Read) {
        let parts = self.0.into_parts();
        let (write_half, read_half) = parts.io.into_split();

        // Create our split read half and populate its buffer with existing contents
        let mut f_read = FramedRead::new(read_half, parts.codec.clone());
        *f_read.read_buffer_mut() = parts.read_buf;

        // Create our split write half and populate its buffer with existing contents
        let mut f_write = FramedWrite::new(write_half, parts.codec);
        *f_write.write_buffer_mut() = parts.write_buf;

        let read_half = FramedTransportReadHalf(f_read);
        let write_half = FramedTransportWriteHalf(f_write);

        (write_half, read_half)
    }
}

#[async_trait]
impl<T, C> UntypedTransportWrite for FramedTransport<T, C>
where
    T: RawTransport + Send,
    C: Codec + Send,
{
    async fn write<D>(&mut self, data: D) -> io::Result<()>
    where
        D: Serialize + Send + 'static,
    {
        // Serialize data into a byte stream
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        let data = utils::serialize_to_vec(&data)?;

        // Use underlying codec to send data (may encrypt, sign, etc.)
        self.0.send(&data).await
    }
}

#[async_trait]
impl<T, C> UntypedTransportRead for FramedTransport<T, C>
where
    T: RawTransport + Send,
    C: Codec + Send,
{
    async fn read<D>(&mut self) -> io::Result<Option<D>>
    where
        D: DeserializeOwned,
    {
        // Use underlying codec to receive data (may decrypt, validate, etc.)
        if let Some(data) = self.0.next().await {
            let data = data?;

            // Deserialize byte stream into our expected type
            let data = utils::deserialize_from_slice(&data)?;
            Ok(Some(data))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InmemoryTransport, PlainCodec};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct TestData {
        name: String,
        value: usize,
    }

    #[tokio::test]
    async fn send_should_convert_data_into_byte_stream_and_send_through_stream() {
        let (_tx, mut rx, stream) = InmemoryTransport::make(1);
        let mut transport = FramedTransport::new(stream, PlainCodec::new());

        let data = TestData {
            name: String::from("test"),
            value: 123,
        };

        let bytes = utils::serialize_to_vec(&data).unwrap();
        let len = (bytes.len() as u64).to_be_bytes();
        let mut frame = Vec::new();
        frame.extend(len.iter().copied());
        frame.extend(bytes);

        transport.write(data).await.unwrap();

        let outgoing = rx.recv().await.unwrap();
        assert_eq!(outgoing, frame);
    }

    #[tokio::test]
    async fn receive_should_return_none_if_stream_is_closed() {
        let (_, _, stream) = InmemoryTransport::make(1);
        let mut transport = FramedTransport::new(stream, PlainCodec::new());

        let result = transport.read::<TestData>().await;
        match result {
            Ok(None) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn receive_should_fail_if_unable_to_convert_to_type() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let mut transport = FramedTransport::new(stream, PlainCodec::new());

        #[derive(Serialize, Deserialize)]
        struct OtherTestData(usize);

        let data = OtherTestData(123);
        let bytes = utils::serialize_to_vec(&data).unwrap();
        let len = (bytes.len() as u64).to_be_bytes();
        let mut frame = Vec::new();
        frame.extend(len.iter().copied());
        frame.extend(bytes);

        tx.send(frame).await.unwrap();
        let result = transport.read::<TestData>().await;
        assert!(result.is_err(), "Unexpectedly succeeded")
    }

    #[tokio::test]
    async fn receive_should_return_some_instance_of_type_when_coming_into_stream() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let mut transport = FramedTransport::new(stream, PlainCodec::new());

        let data = TestData {
            name: String::from("test"),
            value: 123,
        };

        let bytes = utils::serialize_to_vec(&data).unwrap();
        let len = (bytes.len() as u64).to_be_bytes();
        let mut frame = Vec::new();
        frame.extend(len.iter().copied());
        frame.extend(bytes);

        tx.send(frame).await.unwrap();
        let received_data = transport.read::<TestData>().await.unwrap().unwrap();
        assert_eq!(received_data, data);
    }
}
