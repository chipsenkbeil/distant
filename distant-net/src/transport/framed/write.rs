use crate::{transport::framed::utils, Codec};
use async_trait::async_trait;
use futures::SinkExt;
use serde::Serialize;
use std::io;
use tokio::io::AsyncWrite;
use tokio_util::codec::FramedWrite;

/// Interface to write framed data in the form of some seriazable type
#[async_trait]
pub trait FramedTransportWrite {
    /// Sends some data across the wire, waiting for it to completely send
    async fn send<D: Serialize + Send>(&mut self, data: D) -> io::Result<()>;
}

/// Represents a transport of outbound data to the network using frames in order to support
/// typed messages instead of arbitrary bytes being sent across the wire.
///
/// Note that this type does **not** implement [`AsyncWrite`] and instead acts as a
/// wrapper to provide a higher-level interface
pub struct FramedTransportWriteHalf<T, C>(pub(super) FramedWrite<T, C>)
where
    T: AsyncWrite,
    C: Codec;

#[async_trait]
impl<T, C> FramedTransportWrite for FramedTransportWriteHalf<T, C>
where
    T: AsyncWrite + Send + Unpin,
    C: Codec + Send,
{
    async fn send<D: Serialize + Send>(&mut self, data: D) -> io::Result<()> {
        // Serialize data into a byte stream
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        let data = utils::serialize_to_vec(&data)?;

        // Use underlying codec to send data (may encrypt, sign, etc.)
        self.0.send(&data).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FramedTransport, InmemoryTransport, PlainCodec};
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct TestData {
        name: String,
        value: usize,
    }

    #[tokio::test]
    async fn send_should_convert_data_into_byte_stream_and_send_through_stream() {
        let (_tx, mut rx, stream) = InmemoryTransport::make(1);
        let transport = FramedTransport::new(stream, PlainCodec::new());
        let (_, mut wh) = transport.into_split();

        let data = TestData {
            name: String::from("test"),
            value: 123,
        };

        let bytes = utils::serialize_to_vec(&data).unwrap();
        let len = (bytes.len() as u64).to_be_bytes();
        let mut frame = Vec::new();
        frame.extend(len.iter().copied());
        frame.extend(bytes);

        wh.send(data).await.unwrap();

        let outgoing = rx.recv().await.unwrap();
        assert_eq!(outgoing, frame);
    }
}
