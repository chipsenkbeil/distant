use crate::utils::Session;
use codec::{DistantCodec, DistantCodecError};
use derive_more::{Display, Error, From};
use futures::SinkExt;
use orion::{
    aead::{self, SecretKey},
    errors::UnknownCryptoError,
};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;
use tokio::{
    io,
    net::{tcp, TcpStream},
};
use tokio_stream::StreamExt;
use tokio_util::codec::{Framed, FramedRead, FramedWrite};

mod codec;

#[derive(Debug, Display, Error, From)]
pub enum TransportError {
    CodecError(DistantCodecError),
    EncryptError(UnknownCryptoError),
    IoError(io::Error),
    SerializeError(serde_cbor::Error),
}

/// Represents a transport of data across the network
pub struct Transport {
    inner: Framed<TcpStream, DistantCodec>,
    key: Arc<SecretKey>,
}

impl Transport {
    /// Wraps a `TcpStream` and associated credentials in a transport layer
    pub fn new(stream: TcpStream, key: Arc<SecretKey>) -> Self {
        Self {
            inner: Framed::new(stream, DistantCodec),
            key,
        }
    }

    /// Establishes a connection using the provided session
    pub async fn connect(session: Session) -> io::Result<Self> {
        let stream = TcpStream::connect(session.to_socket_addr().await?).await?;
        Ok(Self::new(stream, Arc::new(session.key)))
    }

    /// Sends some data across the wire
    pub async fn send<T: Serialize>(&mut self, data: T) -> Result<(), TransportError> {
        // Serialize, encrypt, and then (TODO) sign
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        let data = serde_cbor::to_vec(&data)?;
        let data = aead::seal(&self.key, &data)?;

        self.inner
            .send(&data)
            .await
            .map_err(TransportError::CodecError)
    }

    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<Option<T>, TransportError> {
        // If data is received, we process like usual
        if let Some(data) = self.inner.next().await {
            // Validate (TODO) signature, decrypt, and then deserialize
            let data = data?;
            let data = aead::open(&self.key, &data)?;
            let data = serde_cbor::from_slice(&data)?;
            Ok(Some(data))

        // Otherwise, if no data is received, this means that our socket has closed
        } else {
            Ok(None)
        }
    }

    /// Splits transport into read and write halves
    pub fn into_split(self) -> (TransportReadHalf, TransportWriteHalf) {
        let key = self.key;
        let parts = self.inner.into_parts();
        let (read_half, write_half) = parts.io.into_split();

        // Create our split read half and populate its buffer with existing contents
        let mut f_read = FramedRead::new(read_half, parts.codec);
        *f_read.read_buffer_mut() = parts.read_buf;

        // Create our split write half and populate its buffer with existing contents
        let mut f_write = FramedWrite::new(write_half, parts.codec);
        *f_write.write_buffer_mut() = parts.write_buf;

        let t_read = TransportReadHalf {
            inner: f_read,
            key: Arc::clone(&key),
        };
        let t_write = TransportWriteHalf {
            inner: f_write,
            key,
        };

        (t_read, t_write)
    }
}

/// Represents a transport of data out to the network
pub struct TransportWriteHalf {
    inner: FramedWrite<tcp::OwnedWriteHalf, DistantCodec>,
    key: Arc<SecretKey>,
}

impl TransportWriteHalf {
    /// Sends some data across the wire
    pub async fn send<T: Serialize>(&mut self, data: T) -> Result<(), TransportError> {
        // Serialize, encrypt, and then (TODO) sign
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        let data = serde_cbor::to_vec(&data)?;
        let data = aead::seal(&self.key, &data)?;

        self.inner
            .send(&data)
            .await
            .map_err(TransportError::CodecError)
    }
}

/// Represents a transport of data in from the network
pub struct TransportReadHalf {
    inner: FramedRead<tcp::OwnedReadHalf, DistantCodec>,
    key: Arc<SecretKey>,
}

impl TransportReadHalf {
    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<Option<T>, TransportError> {
        // If data is received, we process like usual
        if let Some(data) = self.inner.next().await {
            // Validate (TODO) signature, decrypt, and then deserialize
            let data = data?;
            let data = aead::open(&self.key, &data)?;
            let data = serde_cbor::from_slice(&data)?;
            Ok(Some(data))

        // Otherwise, if no data is received, this means that our socket has closed
        } else {
            Ok(None)
        }
    }
}
