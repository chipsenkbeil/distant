use crate::{utils, Codec, IntoSplit, RawTransport, TypedAsyncRead, TypedAsyncWrite};
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

/// Represents a transport of data across the network using frames in order to support
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
impl<T, C, D> TypedAsyncWrite<D> for FramedTransport<T, C>
where
    T: RawTransport + Send,
    C: Codec + Send,
    D: Serialize + Send + 'static,
{
    async fn write(&mut self, data: D) -> io::Result<()> {
        // Serialize data into a byte stream
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        let data = utils::serialize_to_vec(&data)?;

        // Use underlying codec to send data (may encrypt, sign, etc.)
        self.0.send(&data).await
    }
}

#[async_trait]
impl<T, C, D> TypedAsyncRead<D> for FramedTransport<T, C>
where
    T: RawTransport + Send,
    C: Codec + Send,
    D: DeserializeOwned,
{
    async fn read(&mut self) -> io::Result<Option<D>> {
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

        let result = TypedAsyncRead::<TestData>::read(&mut transport).await;
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
        let result = TypedAsyncRead::<TestData>::read(&mut transport).await;
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
        let received_data = TypedAsyncRead::<TestData>::read(&mut transport)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received_data, data);
    }
}
