use crate::{constants::SALT_LEN, utils::Session};
use codec::DistantCodec;
use derive_more::{Display, Error, From};
use futures::SinkExt;
use k256::{ecdh::EphemeralSecret, EncodedPoint, PublicKey};
use orion::{
    aead::{self, SecretKey},
    auth::{self, Tag},
    errors::UnknownCryptoError,
    kdf::{self, Salt},
    pwhash::Password,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    io,
    net::{tcp, TcpStream},
};
use tokio_stream::StreamExt;
use tokio_util::codec::{Framed, FramedRead, FramedWrite};

mod codec;

#[derive(Debug, Display, Error, From)]
pub enum TransportError {
    #[from(ignore)]
    AuthError(UnknownCryptoError),
    #[from(ignore)]
    EncryptError(UnknownCryptoError),
    IoError(io::Error),
    SerializeError(serde_cbor::Error),
}

/// Represents a transport of data across the network
pub struct Transport {
    /// Underlying connection to some remote system
    conn: Framed<TcpStream, DistantCodec>,

    /// Used to sign and validate messages
    auth_key: Arc<SecretKey>,

    /// Used to encrypt and decrypt messages
    crypt_key: Arc<SecretKey>,
}

impl Transport {
    /// Takes a pre-existing connection and performs a handshake to build out the encryption key
    /// with the remote system, returning a transport ready to communicate with the other side
    pub async fn from_handshake(stream: TcpStream, auth_key: Arc<SecretKey>) -> io::Result<Self> {
        // First, wrap the raw stream in our framed codec
        let mut conn = Framed::new(stream, DistantCodec);

        // Second, generate a private key that will be used to eventually derive a shared secret
        let private_key = EphemeralSecret::random(&mut rand::rngs::OsRng);

        // Third, produce a private key that will be shared unencrypted to the other side
        let public_key = EncodedPoint::from(private_key.public_key());

        // Fourth, share a random salt and the public key with the server as our first message
        let salt = Salt::generate(SALT_LEN).map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
        let mut data = Vec::new();
        data.extend_from_slice(salt.as_ref());
        data.extend_from_slice(public_key.as_bytes());
        conn.send(&data)
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

        // Fifth, wait for a response that we will assume is the other side's salt & public key
        let data = conn.next().await.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Stream ended before handshake completed",
            )
        })??;
        let (salt_bytes, other_public_key_bytes) = data.split_at(SALT_LEN);
        let other_salt = Salt::from_slice(salt_bytes)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Sixth, decode the serialized public key from the other side
        let other_public_key = PublicKey::from_sec1_bytes(other_public_key_bytes)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Seventh, establish a shared secret that is NOT uniformly random, so we can't
        // directly use it as our encryption key (32 bytes in length)
        let shared_secret = private_key.diffie_hellman(&other_public_key);

        // Eighth, convert our secret key into an orion password that we'll use to derive
        // a new key; need to ensure that the secret is at least 32 bytes!
        let password = Password::from_slice(shared_secret.as_bytes())
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Ninth, take our salt and the salt from the other side and combine them in a consistent
        // manner such that both sides derive the same salt
        let mixed_salt = Salt::from_slice(
            &salt
                .as_ref()
                .iter()
                .zip(other_salt.as_ref().iter())
                .map(|(x, y)| x ^ y)
                .collect::<Vec<u8>>(),
        )
        .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Tenth, derive a higher-entropy key from our shared secret
        let derived_key = kdf::derive_key(&password, &mixed_salt, 3, 1 << 16, 32)
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

        let crypt_key = Arc::new(derived_key);
        log::trace!(
            "Handshake complete: {}",
            hex::encode(crypt_key.unprotected_as_bytes())
        );

        Ok(Self {
            conn,
            auth_key,
            crypt_key,
        })
    }

    /// Establishes a connection using the provided session and performs a handshake to establish
    /// means of encryption, returning a transport ready to communicate with the other side
    pub async fn connect(session: Session) -> io::Result<Self> {
        let stream = TcpStream::connect(session.to_socket_addr().await?).await?;
        Self::from_handshake(stream, Arc::new(session.auth_key)).await
    }

    /// Returns the address of the peer the transport is connected to
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.conn.get_ref().peer_addr()
    }

    /// Splits transport into read and write halves
    pub fn into_split(self) -> (TransportReadHalf, TransportWriteHalf) {
        let auth_key = self.auth_key;
        let crypt_key = self.crypt_key;
        let parts = self.conn.into_parts();
        let (read_half, write_half) = parts.io.into_split();

        // Create our split read half and populate its buffer with existing contents
        let mut f_read = FramedRead::new(read_half, parts.codec);
        *f_read.read_buffer_mut() = parts.read_buf;

        // Create our split write half and populate its buffer with existing contents
        let mut f_write = FramedWrite::new(write_half, parts.codec);
        *f_write.write_buffer_mut() = parts.write_buf;

        let t_read = TransportReadHalf {
            conn: f_read,
            auth_key: Arc::clone(&auth_key),
            crypt_key: Arc::clone(&crypt_key),
        };
        let t_write = TransportWriteHalf {
            conn: f_write,
            auth_key,
            crypt_key,
        };

        (t_read, t_write)
    }
}

/// Represents a transport of data out to the network
pub struct TransportWriteHalf {
    /// Underlying connection to some remote system
    conn: FramedWrite<tcp::OwnedWriteHalf, DistantCodec>,

    /// Used to sign and validate messages
    auth_key: Arc<SecretKey>,

    /// Used to encrypt and decrypt messages
    crypt_key: Arc<SecretKey>,
}

impl TransportWriteHalf {
    /// Sends some data across the wire, waiting for it to completely send
    pub async fn send<T: Serialize>(&mut self, data: T) -> Result<(), TransportError> {
        // Serialize, encrypt, and then sign
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        let data = serde_cbor::to_vec(&data)?;
        let data = aead::seal(&self.crypt_key, &data).map_err(TransportError::EncryptError)?;
        let tag = auth::authenticate(&self.auth_key, &data).map_err(TransportError::AuthError)?;

        // Send {TAG LEN}{TAG}{ENCRYPTED DATA}
        let mut out: Vec<u8> = Vec::new();
        out.push(tag.unprotected_as_bytes().len() as u8);
        out.extend_from_slice(tag.unprotected_as_bytes());
        out.extend(data);
        self.conn.send(&out).await.map_err(TransportError::from)
    }
}

/// Represents a transport of data in from the network
pub struct TransportReadHalf {
    /// Underlying connection to some remote system
    conn: FramedRead<tcp::OwnedReadHalf, DistantCodec>,

    /// Used to sign and validate messages
    auth_key: Arc<SecretKey>,

    /// Used to encrypt and decrypt messages
    crypt_key: Arc<SecretKey>,
}

impl TransportReadHalf {
    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<Option<T>, TransportError> {
        // If data is received, we process like usual
        if let Some(data) = self.conn.next().await {
            let mut data = data?;
            if data.is_empty() {
                return Err(TransportError::from(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Received data is empty",
                )));
            }

            // Retrieve in form {TAG LEN}{TAG}{ENCRYPTED DATA}
            let tag_len = data[0];
            let tag =
                Tag::from_slice(&data[1..=tag_len as usize]).map_err(TransportError::AuthError)?;
            let data = data.split_off(tag_len as usize + 1);

            // Validate signature, decrypt, and then deserialize
            auth::authenticate_verify(&tag, &self.auth_key, &data)
                .map_err(TransportError::AuthError)?;
            let data = aead::open(&self.crypt_key, &data).map_err(TransportError::EncryptError)?;
            let data = serde_cbor::from_slice(&data)?;
            Ok(Some(data))

        // Otherwise, if no data is received, this means that our socket has closed
        } else {
            Ok(None)
        }
    }
}
