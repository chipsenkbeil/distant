use crate::net::SecretKeyError;
use derive_more::{Display, Error, From};
use futures::{SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use std::marker::Unpin;
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, FramedRead, FramedWrite};

mod codec;
pub use codec::*;

mod inmemory;
pub use inmemory::*;

mod tcp;
pub use tcp::*;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

#[derive(Debug, Display, Error, From)]
pub enum TransportError {
    CryptoError(SecretKeyError),
    IoError(io::Error),
    SerializeError(serde_cbor::Error),
}

/// Interface representing a two-way data stream
///
/// Enables splitting into separate, functioning halves that can read and write respectively
pub trait DataStream: AsyncRead + AsyncWrite + Unpin {
    type Read: AsyncRead + Send + Unpin + 'static;
    type Write: AsyncWrite + Send + Unpin + 'static;

    /// Returns a textual description of the connection associated with this stream
    fn to_connection_tag(&self) -> String;

    /// Splits this stream into read and write halves
    fn into_split(self) -> (Self::Read, Self::Write);
}

/// Represents a transport of data across the network
#[derive(Debug)]
pub struct Transport<T, U>(Framed<T, U>)
where
    T: DataStream,
    U: Codec;

impl<T, U> Transport<T, U>
where
    T: DataStream,
    U: Codec,
{
    /// Creates a new instance of the transport, wrapping the stream in a `Framed<T, XChaCha20Poly1305Codec>`
    pub fn new(stream: T, codec: U) -> Self {
        Self(Framed::new(stream, codec))
    }

    /// Sends some data across the wire, waiting for it to completely send
    pub async fn send<D: Serialize>(&mut self, data: D) -> Result<(), TransportError> {
        // Serialize data into a byte stream
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        let data = serde_cbor::to_vec(&data)?;

        // Use underlying codec to send data (may encrypt, sign, etc.)
        self.0.send(&data).await.map_err(TransportError::from)
    }

    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    pub async fn receive<R: DeserializeOwned>(&mut self) -> Result<Option<R>, TransportError> {
        // Use underlying codec to receive data (may decrypt, validate, etc.)
        if let Some(data) = self.0.next().await {
            let data = data?;

            if data.is_empty() {
                return Err(TransportError::from(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Received data is empty",
                )));
            }

            // Deserialize byte stream into our expected type
            let data = serde_cbor::from_slice(&data)?;
            Ok(Some(data))
        } else {
            Ok(None)
        }
    }

    /// Returns a textual description of the transport's underlying connection
    pub fn to_connection_tag(&self) -> String {
        self.0.get_ref().to_connection_tag()
    }

    /// Splits transport into read and write halves
    pub fn into_split(
        self,
    ) -> (
        TransportReadHalf<T::Read, U>,
        TransportWriteHalf<T::Write, U>,
    ) {
        let parts = self.0.into_parts();
        let (read_half, write_half) = parts.io.into_split();

        // Create our split read half and populate its buffer with existing contents
        let mut f_read = FramedRead::new(read_half, parts.codec.clone());
        *f_read.read_buffer_mut() = parts.read_buf;

        // Create our split write half and populate its buffer with existing contents
        let mut f_write = FramedWrite::new(write_half, parts.codec);
        *f_write.write_buffer_mut() = parts.write_buf;

        let t_read = TransportReadHalf(f_read);
        let t_write = TransportWriteHalf(f_write);

        (t_read, t_write)
    }
}

/// Represents a transport of data out to the network
pub struct TransportWriteHalf<T, U>(FramedWrite<T, U>)
where
    T: AsyncWrite + Unpin,
    U: Codec;

impl<T, U> TransportWriteHalf<T, U>
where
    T: AsyncWrite + Unpin,
    U: Codec,
{
    /// Sends some data across the wire, waiting for it to completely send
    pub async fn send<D: Serialize>(&mut self, data: D) -> Result<(), TransportError> {
        // Serialize data into a byte stream
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        let data = serde_cbor::to_vec(&data)?;

        // Use underlying codec to send data (may encrypt, sign, etc.)
        self.0.send(&data).await.map_err(TransportError::from)
    }
}

/// Represents a transport of data in from the network
pub struct TransportReadHalf<T, U>(FramedRead<T, U>)
where
    T: AsyncRead + Unpin,
    U: Codec;

impl<T, U> TransportReadHalf<T, U>
where
    T: AsyncRead + Unpin,
    U: Codec,
{
    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    pub async fn receive<R: DeserializeOwned>(&mut self) -> Result<Option<R>, TransportError> {
        // Use underlying codec to receive data (may decrypt, validate, etc.)
        if let Some(data) = self.0.next().await {
            let data = data?;

            if data.is_empty() {
                return Err(TransportError::from(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Received data is empty",
                )));
            }

            // Deserialize byte stream into our expected type
            let data = serde_cbor::from_slice(&data)?;
            Ok(Some(data))
        } else {
            Ok(None)
        }
    }
}

/// Test utilities
#[cfg(test)]
impl Transport<crate::net::InmemoryStream, crate::net::PlainCodec> {
    /// Makes a connected pair of inmemory transports
    pub fn make_pair() -> (
        Transport<crate::net::InmemoryStream, crate::net::PlainCodec>,
        Transport<crate::net::InmemoryStream, crate::net::PlainCodec>,
    ) {
        Self::pair(crate::constants::test::BUFFER_SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct TestData {
        name: &'static str,
        value: usize,
    }

    #[test]
    fn send_should_convert_data_into_byte_stream_and_send_through_stream() {
        todo!();
    }

    #[test]
    fn receive_should_fail_if_received_data_is_empty() {
        todo!();
    }

    #[test]
    fn receive_should_return_none_if_stream_is_closed() {
        todo!();
    }

    #[test]
    fn receive_should_fail_if_unable_to_convert_to_type() {
        todo!();
    }

    #[test]
    fn receive_should_return_some_instance_of_type_when_coming_into_stream() {
        todo!();
    }

    #[test]
    fn to_connection_tag_should_return_connection_tag_of_stream() {
        todo!();
    }
}
