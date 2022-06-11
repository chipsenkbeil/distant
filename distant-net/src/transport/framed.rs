use crate::{Codec, Transport};
use futures::{SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::codec::{Framed, FramedRead, FramedWrite};

#[cfg(test)]
mod test;

#[cfg(test)]
pub use test::*;

mod read;
pub use read::*;

mod write;
pub use write::*;

mod utils;

/// Represents a transport of data across the network using frames
#[derive(Debug)]
pub struct FramedTransport<T, U>(Framed<T, U>)
where
    T: Transport,
    U: Codec;

impl<T, U> FramedTransport<T, U>
where
    T: Transport,
    U: Codec,
{
    /// Creates a new instance of the transport, wrapping the stream in a `Framed<T, XChaCha20Poly1305Codec>`
    pub fn new(transport: T, codec: U) -> Self {
        Self(Framed::new(transport, codec))
    }

    /// Returns a reference to the underlying I/O stream
    ///
    /// Note that care should be taken to not tamper with the underlying stream of data coming in
    /// as it may corrupt the stream of frames otherwise being worked with
    pub fn get_ref(&self) -> &T {
        self.0.get_ref()
    }

    /// Returns a reference to the underlying I/O stream
    ///
    /// Note that care should be taken to not tamper with the underlying stream of data coming in
    /// as it may corrupt the stream of frames otherwise being worked with
    pub fn get_mut(&mut self) -> &mut T {
        self.0.get_mut()
    }

    /// Consumes the transport, returning its underlying I/O stream
    ///
    /// Note that care should be taken to not tamper with the underlying stream of data coming in
    /// as it may corrupt the stream of frames otherwise being worked with.
    pub fn into_inner(self) -> T {
        self.0.into_inner()
    }
}

impl<T, U> Transport for FramedTransport<T, U>
where
    T: Transport,
    U: Codec + Send + 'static,
{
    type ReadHalf = FramedTransportReadHalf<T::ReadHalf, U>;
    type WriteHalf = FramedTransportWriteHalf<T::WriteHalf, U>;

    fn into_split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        let parts = self.0.into_parts();
        let (read_half, write_half) = parts.io.into_split();

        // Create our split read half and populate its buffer with existing contents
        let mut f_read = FramedRead::new(read_half, parts.codec.clone());
        *f_read.read_buffer_mut() = parts.read_buf;

        // Create our split write half and populate its buffer with existing contents
        let mut f_write = FramedWrite::new(write_half, parts.codec);
        *f_write.write_buffer_mut() = parts.write_buf;

        let read_half = FramedTransportReadHalf(f_read);
        let write_half = FramedTransportWriteHalf(f_write);

        (read_half, write_half)
    }
}

impl<T, U> AsyncRead for FramedTransport<T, U>
where
    T: Transport,
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

impl<T, U> AsyncWrite for FramedTransport<T, U>
where
    T: Transport,
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

impl<T, U> FramedTransport<T, U>
where
    T: Transport,
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

impl<T, U> FramedTransport<T, U>
where
    T: Transport,
    U: Codec,
{
    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    pub async fn receive<D: DeserializeOwned>(&mut self) -> io::Result<Option<D>> {
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

        transport.send(data).await.unwrap();

        let outgoing = rx.recv().await.unwrap();
        assert_eq!(outgoing, frame);
    }

    #[tokio::test]
    async fn receive_should_return_none_if_stream_is_closed() {
        let (_, _, stream) = InmemoryTransport::make(1);
        let mut transport = FramedTransport::new(stream, PlainCodec::new());

        let result = transport.receive::<TestData>().await;
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
        let result = transport.receive::<TestData>().await;
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
        let received_data = transport.receive::<TestData>().await.unwrap().unwrap();
        assert_eq!(received_data, data);
    }
}
