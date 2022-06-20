use crate::{transport::framed::utils, Codec, SerdeTransportRead};
use async_trait::async_trait;
use futures::StreamExt;
use serde::de::DeserializeOwned;
use std::io;
use tokio::io::AsyncRead;
use tokio_util::codec::FramedRead;

/// Represents a transport of inbound data from the network using frames in order to support
/// typed messages instead of arbitrary bytes being sent across the wire.
///
/// Note that this type does **not** implement [`AsyncRead`] and instead acts as a
/// wrapper to provide a higher-level interface
pub struct FramedTransportReadHalf<R, C>(pub(super) FramedRead<R, C>)
where
    R: AsyncRead,
    C: Codec;

#[async_trait]
impl<R, C> SerdeTransportRead for FramedTransportReadHalf<R, C>
where
    R: AsyncRead + Send + Unpin,
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
    use crate::{FramedTransport, InmemoryTransport, IntoSplit, PlainCodec};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct TestData {
        name: String,
        value: usize,
    }

    #[tokio::test]
    async fn receive_should_return_none_if_stream_is_closed() {
        let (_, _, stream) = InmemoryTransport::make(1);
        let transport = FramedTransport::new(stream, PlainCodec::new());
        let (_, mut reader) = transport.into_split();

        let result = reader.read::<TestData>().await;
        match result {
            Ok(None) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn receive_should_fail_if_unable_to_convert_to_type() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let transport = FramedTransport::new(stream, PlainCodec::new());
        let (_, mut reader) = transport.into_split();

        #[derive(Serialize, Deserialize)]
        struct OtherTestData(usize);

        let data = OtherTestData(123);
        let bytes = utils::serialize_to_vec(&data).unwrap();
        let len = (bytes.len() as u64).to_be_bytes();
        let mut frame = Vec::new();
        frame.extend(len.iter().copied());
        frame.extend(bytes);

        tx.send(frame).await.unwrap();
        let result = reader.read::<TestData>().await;
        assert!(result.is_err(), "Unexpectedly succeeded");
    }

    #[tokio::test]
    async fn receive_should_return_some_instance_of_type_when_coming_into_stream() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let transport = FramedTransport::new(stream, PlainCodec::new());
        let (_, mut reader) = transport.into_split();

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
        let received_data = reader.read::<TestData>().await.unwrap().unwrap();
        assert_eq!(received_data, data);
    }
}
