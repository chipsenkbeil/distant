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
use tokio::{io, net::TcpStream};
use tokio_stream::StreamExt;
use tokio_util::codec::Framed;

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
        let data = serde_cbor::ser::to_vec_packed(&data)?;
        let data = aead::seal(&self.key, &data)?;

        self.inner
            .send(&data)
            .await
            .map_err(TransportError::CodecError)
    }

    /// Receives some data from out on the wire, waiting until it's available
    pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<T, TransportError> {
        loop {
            if let Some(data) = self.try_receive().await? {
                break Ok(data);
            }
        }
    }

    /// Attempts to receive some data from out on the wire, returning that data if available
    /// or none if unavailable
    pub async fn try_receive<T: DeserializeOwned>(&mut self) -> Result<Option<T>, TransportError> {
        if let Some(data) = self.inner.next().await {
            // Validate (TODO), decrypt, and then deserialize
            let data = data?;
            let data = aead::open(&self.key, &data)?;
            let data = serde_cbor::from_slice(&data)?;
            Ok(Some(data))
        } else {
            Ok(None)
        }
    }
}
