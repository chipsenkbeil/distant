use crate::{transport::framed::utils, Codec};
use futures::StreamExt;
use serde::de::DeserializeOwned;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, ReadBuf};
use tokio_util::codec::FramedRead;

/// Represents a transport of data in from the network
pub struct FramedTransportReadHalf<T, U>(pub(super) FramedRead<T, U>)
where
    T: AsyncRead,
    U: Codec;

impl<T, U> FramedTransportReadHalf<T, U>
where
    T: AsyncRead + Unpin,
    U: Codec,
{
    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    pub async fn receive<R: DeserializeOwned>(&mut self) -> io::Result<Option<R>> {
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

impl<T, U> AsyncRead for FramedTransportReadHalf<T, U>
where
    T: AsyncRead + Unpin,
    U: Codec,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(self.0.get_mut()).poll_read(cx, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FramedTransport, InmemoryTransport, PlainCodec, Transport};
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
        let (mut rh, _) = transport.into_split();

        let result = rh.receive::<TestData>().await;
        match result {
            Ok(None) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn receive_should_fail_if_unable_to_convert_to_type() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let transport = FramedTransport::new(stream, PlainCodec::new());
        let (mut rh, _) = transport.into_split();

        #[derive(Serialize, Deserialize)]
        struct OtherTestData(usize);

        let data = OtherTestData(123);
        let bytes = utils::serialize_to_vec(&data).unwrap();
        let len = (bytes.len() as u64).to_be_bytes();
        let mut frame = Vec::new();
        frame.extend(len.iter().copied());
        frame.extend(bytes);

        tx.send(frame).await.unwrap();
        let result = rh.receive::<TestData>().await;
        assert!(result.is_err(), "Unexpectedly succeeded");
    }

    #[tokio::test]
    async fn receive_should_return_some_instance_of_type_when_coming_into_stream() {
        let (tx, _rx, stream) = InmemoryTransport::make(1);
        let transport = FramedTransport::new(stream, PlainCodec::new());
        let (mut rh, _) = transport.into_split();

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
        let received_data = rh.receive::<TestData>().await.unwrap().unwrap();
        assert_eq!(received_data, data);
    }
}
