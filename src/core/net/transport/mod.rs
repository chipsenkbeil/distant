use crate::core::{constants::SALT_LEN, session::Session};
use codec::DistantCodec;
use derive_more::{Display, Error, From};
use futures::SinkExt;
use k256::{ecdh::EphemeralSecret, EncodedPoint, PublicKey};
use log::*;
use orion::{
    aead::{self, SecretKey},
    auth::{self, Tag},
    errors::UnknownCryptoError,
    kdf::{self, Salt},
    pwhash::Password,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{marker::Unpin, net::SocketAddr, sync::Arc};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    net::{self, tcp, TcpStream},
};
use tokio_stream::StreamExt;
use tokio_util::codec::{Framed, FramedRead, FramedWrite};

mod codec;

#[derive(Debug, Display, Error, From)]
pub enum TransportError {
    #[from(ignore)]
    #[display(fmt = "Authentication Error: {}", _0)]
    AuthError(UnknownCryptoError),
    #[from(ignore)]
    #[display(fmt = "Encryption Error: {}", _0)]
    EncryptError(UnknownCryptoError),
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

impl DataStream for TcpStream {
    type Read = tcp::OwnedReadHalf;
    type Write = tcp::OwnedWriteHalf;

    fn to_connection_tag(&self) -> String {
        self.peer_addr()
            .map(|addr| format!("{}", addr))
            .unwrap_or_else(|_| String::from("--"))
    }

    fn into_split(self) -> (Self::Read, Self::Write) {
        TcpStream::into_split(self)
    }
}

#[cfg(unix)]
impl DataStream for net::UnixStream {
    type Read = net::unix::OwnedReadHalf;
    type Write = net::unix::OwnedWriteHalf;

    fn to_connection_tag(&self) -> String {
        self.peer_addr()
            .map(|addr| format!("{:?}", addr))
            .unwrap_or_else(|_| String::from("--"))
    }

    fn into_split(self) -> (Self::Read, Self::Write) {
        net::UnixStream::into_split(self)
    }
}

/// Represents a transport of data across the network
pub struct Transport<T>
where
    T: DataStream,
{
    /// Underlying connection to some remote system
    conn: Framed<T, DistantCodec>,

    /// Used to sign and validate messages
    auth_key: Option<Arc<SecretKey>>,

    /// Used to encrypt and decrypt messages
    crypt_key: Arc<SecretKey>,
}

impl<T> Transport<T>
where
    T: DataStream,
{
    /// Takes a pre-existing connection and performs a handshake to build out the encryption key
    /// with the remote system, returning a transport ready to communicate with the other side
    ///
    /// An optional authentication key can be provided that will be used alongside encryption
    /// when communicating across the wire
    pub async fn from_handshake(stream: T, auth_key: Option<Arc<SecretKey>>) -> io::Result<Self> {
        let connection_tag = stream.to_connection_tag();
        trace!("Beginning handshake for {}", connection_tag);

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
        trace!("Finished handshake for {}", connection_tag);

        Ok(Self {
            conn,
            auth_key,
            crypt_key,
        })
    }
}

impl<T> Transport<T>
where
    T: AsyncRead + AsyncWrite + DataStream + Unpin,
{
    /// Splits transport into read and write halves
    pub fn into_split(self) -> (TransportReadHalf<T::Read>, TransportWriteHalf<T::Write>) {
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
            auth_key: self.auth_key.as_ref().map(Arc::clone),
            crypt_key: Arc::clone(&crypt_key),
        };
        let t_write = TransportWriteHalf {
            conn: f_write,
            auth_key: self.auth_key.as_ref().map(Arc::clone),
            crypt_key,
        };

        (t_read, t_write)
    }
}

impl Transport<TcpStream> {
    /// Establishes a connection using the provided session and performs a handshake to establish
    /// means of encryption, returning a transport ready to communicate with the other side
    ///
    /// TCP Streams will always use a session's authentication key
    pub async fn connect(session: Session) -> io::Result<Self> {
        let stream = TcpStream::connect(session.to_socket_addr().await?).await?;
        Self::from_handshake(stream, Some(Arc::new(session.auth_key))).await
    }

    /// Returns the address of the peer the transport is connected to
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.conn.get_ref().peer_addr()
    }
}

#[cfg(unix)]
impl Transport<net::UnixStream> {
    /// Establishes a connection using the provided session and performs a handshake to establish
    /// means of encryption, returning a transport ready to communicate with the other side
    ///
    /// Takes an optional authentication key
    pub async fn connect(
        path: impl AsRef<std::path::Path>,
        auth_key: Option<Arc<SecretKey>>,
    ) -> io::Result<Self> {
        let stream = net::UnixStream::connect(path.as_ref()).await?;
        Self::from_handshake(stream, auth_key).await
    }

    /// Returns the address of the peer the transport is connected to
    pub fn peer_addr(&self) -> io::Result<net::unix::SocketAddr> {
        self.conn.get_ref().peer_addr()
    }
}

/// Represents a transport of data out to the network
pub struct TransportWriteHalf<T>
where
    T: AsyncWrite + Unpin,
{
    /// Underlying connection to some remote system
    conn: FramedWrite<T, DistantCodec>,

    /// Used to sign and validate messages; if none then no sign/validation occurs
    auth_key: Option<Arc<SecretKey>>,

    /// Used to encrypt and decrypt messages
    crypt_key: Arc<SecretKey>,
}

impl<T> TransportWriteHalf<T>
where
    T: AsyncWrite + Unpin,
{
    /// Sends some data across the wire, waiting for it to completely send
    pub async fn send<D: Serialize>(&mut self, data: D) -> Result<(), TransportError> {
        // Serialize, encrypt, and then sign
        // NOTE: Cannot used packed implementation for now due to issues with deserialization
        trace!("Serializing data");
        let data = serde_cbor::to_vec(&data)?;

        trace!("Encrypting data of len {}", data.len());
        let data = aead::seal(&self.crypt_key, &data).map_err(TransportError::EncryptError)?;
        let tag = self
            .auth_key
            .as_ref()
            .map(|key| auth::authenticate(key, &data))
            .transpose()
            .map_err(TransportError::AuthError)?;

        // Send {TAG LEN}{TAG}{ENCRYPTED DATA} if we have an auth key,
        // otherwise just send the encrypted data on its own
        let mut out: Vec<u8> = Vec::new();
        if let Some(tag) = tag {
            trace!("Signing data of len {}", data.len());
            let tag_len = tag.unprotected_as_bytes().len() as u8;

            trace!("Tag len {}", tag_len);
            out.push(tag_len);
            out.extend_from_slice(tag.unprotected_as_bytes());
        }
        out.extend(data);

        trace!("Sending out data of len {}", out.len());
        self.conn.send(&out).await.map_err(TransportError::from)
    }
}

/// Represents a transport of data in from the network
pub struct TransportReadHalf<T>
where
    T: AsyncRead + Unpin,
{
    /// Underlying connection to some remote system
    conn: FramedRead<T, DistantCodec>,

    /// Used to sign and validate messages
    auth_key: Option<Arc<SecretKey>>,

    /// Used to encrypt and decrypt messages
    crypt_key: Arc<SecretKey>,
}

impl<T> TransportReadHalf<T>
where
    T: AsyncRead + Unpin,
{
    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    pub async fn receive<R: DeserializeOwned>(&mut self) -> Result<Option<R>, TransportError> {
        // If data is received, we process like usual
        if let Some(data) = self.conn.next().await {
            let mut data = data?;

            trace!("Received data of len {}", data.len());
            if data.is_empty() {
                return Err(TransportError::from(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Received data is empty",
                )));
            }

            // Retrieve in form {TAG LEN}{TAG}{ENCRYPTED DATA}
            // with the tag len and tag being optional
            if let Some(auth_key) = self.auth_key.as_ref() {
                trace!("Verifying signature on data of len {}", data.len());

                // Parse the tag from the length, protecting against bad lengths
                let tag_len = data[0];
                if data.len() <= tag_len as usize {
                    return Err(TransportError::from(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Tag len {} > Data len {}", tag_len, data.len()),
                    )));
                }

                trace!("Tag len {}", tag_len);
                let tag = Tag::from_slice(&data[1..=tag_len as usize])
                    .map_err(TransportError::AuthError)?;

                // Update data with the content after the tag by mutating
                // the current data to point to the return from split_off
                data = data.split_off(tag_len as usize + 1);

                // Validate signature, decrypt, and then deserialize
                auth::authenticate_verify(&tag, auth_key, &data)
                    .map_err(TransportError::AuthError)?;
            }

            trace!("Decrypting data of len {}", data.len());
            let data = aead::open(&self.crypt_key, &data).map_err(TransportError::EncryptError)?;

            trace!("Deserializing decrypted data of len {}", data.len());
            let data = serde_cbor::from_slice(&data)?;
            Ok(Some(data))

        // Otherwise, if no data is received, this means that our socket has closed
        } else {
            Ok(None)
        }
    }
}
