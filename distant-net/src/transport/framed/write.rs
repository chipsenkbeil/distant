use crate::{transport::framed::utils, Codec};
use futures::SinkExt;
use serde::Serialize;
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::AsyncWrite;
use tokio_util::codec::FramedWrite;

/// Represents a transport of data out to the network
pub struct FramedTransportWriteHalf<T, U>(pub(super) FramedWrite<T, U>)
where
    T: AsyncWrite,
    U: Codec;

impl<T, U> FramedTransportWriteHalf<T, U>
where
    T: AsyncWrite + Unpin,
    U: Codec,
{
    /// Sends some data across the wire, waiting for it to completely send
    pub async fn send<D: Serialize>(&mut self, data: D) -> io::Result<()> {
        // Serialize data into a byte stream
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        let data = utils::serialize_to_vec(&data)?;

        // Use underlying codec to send data (may encrypt, sign, etc.)
        self.0.send(&data).await
    }
}

impl<T, U> AsyncWrite for FramedTransportWriteHalf<T, U>
where
    T: AsyncWrite + Unpin,
    U: Codec,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(self.0.get_mut()).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(self.0.get_mut()).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(self.0.get_mut()).poll_shutdown(cx)
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
