use crate::{constants::SALT_LEN, net::SecretKey};
use codec::DistantCodec;
use derive_more::{Display, Error, From};
use futures::SinkExt;
use futures::StreamExt;
use k256::{ecdh::EphemeralSecret, EncodedPoint, PublicKey};
use log::*;
use orion::{
    aead,
    auth::{self, Tag},
    errors::UnknownCryptoError,
    kdf::{self, Salt},
    pwhash::Password,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{marker::Unpin, sync::Arc};
use tokio::io::{self, AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, FramedRead, FramedWrite};

mod codec;

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

/// Sends some data across the wire, waiting for it to completely send
macro_rules! send {
    ($conn:expr, $crypt_key:expr, $auth_key:expr, $data:expr) => {
        async {
            // Serialize, encrypt, and then sign
            // NOTE: Cannot used packed implementation for now due to issues with deserialization
            let data = serde_cbor::to_vec(&$data)?;

            let data = aead::seal(&$crypt_key, &data).map_err(TransportError::EncryptError)?;
            let tag = $auth_key
                .as_ref()
                .map(|key| auth::authenticate(key, &data))
                .transpose()
                .map_err(TransportError::AuthError)?;

            // Send {TAG LEN}{TAG}{ENCRYPTED DATA} if we have an auth key,
            // otherwise just send the encrypted data on its own
            let mut out: Vec<u8> = Vec::new();
            if let Some(tag) = tag {
                let tag_len = tag.unprotected_as_bytes().len() as u8;

                out.push(tag_len);
                out.extend_from_slice(tag.unprotected_as_bytes());
            }
            out.extend(data);

            $conn.send(&out).await.map_err(TransportError::from)
        }
    };
}

macro_rules! recv {
    ($conn:expr, $crypt_key:expr, $auth_key:expr) => {
        async {
            // If data is received, we process like usual
            if let Some(data) = $conn.next().await {
                let mut data = data?;

                if data.is_empty() {
                    return Err(TransportError::from(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Received data is empty",
                    )));
                }

                // Retrieve in form {TAG LEN}{TAG}{ENCRYPTED DATA}
                // with the tag len and tag being optional
                if let Some(auth_key) = $auth_key.as_ref() {
                    // Parse the tag from the length, protecting against bad lengths
                    let tag_len = data[0];
                    if data.len() <= tag_len as usize {
                        return Err(TransportError::from(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Tag len {} > Data len {}", tag_len, data.len()),
                        )));
                    }

                    let tag = Tag::from_slice(&data[1..=tag_len as usize])
                        .map_err(TransportError::AuthError)?;

                    // Update data with the content after the tag by mutating
                    // the current data to point to the return from split_off
                    data = data.split_off(tag_len as usize + 1);

                    // Validate signature, decrypt, and then deserialize
                    auth::authenticate_verify(&tag, auth_key, &data)
                        .map_err(TransportError::AuthError)?;
                }

                let data = aead::open(&$crypt_key, &data).map_err(TransportError::EncryptError)?;

                let data = serde_cbor::from_slice(&data)?;
                Ok(Some(data))

            // Otherwise, if no data is received, this means that our socket has closed
            } else {
                Ok(None)
            }
        }
    };
}

/// Represents a transport of data across the network
#[derive(Debug)]
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
    /// Creates a new instance of the transport, wrapping the stream in a `Framed<T, DistantCodec>`
    pub fn new(stream: T, auth_key: Option<Arc<SecretKey>>, crypt_key: Arc<SecretKey>) -> Self {
        Self {
            conn: Framed::new(stream, DistantCodec),
            auth_key,
            crypt_key,
        }
    }

    /// Takes a pre-existing connection and performs a handshake to build out the encryption key
    /// with the remote system, returning a transport ready to communicate with the other side
    ///
    /// An optional authentication key can be provided that will be used alongside encryption
    /// when communicating across the wire
    pub async fn from_handshake(stream: T, auth_key: Option<Arc<SecretKey>>) -> io::Result<Self> {
        let connection_tag = stream.to_connection_tag();
        trace!("Beginning handshake with {}", connection_tag);

        // First, wrap the raw stream in our framed codec
        let mut conn = Framed::new(stream, DistantCodec);

        // Second, generate a private key that will be used to eventually derive a shared secret
        let private_key = EphemeralSecret::random(&mut rand::rngs::OsRng);

        // Third, produce a private key that will be shared unencrypted to the other side
        let public_key = EncodedPoint::from(private_key.public_key());

        // Fourth, share a random salt and the public key with the server as our first message
        trace!("Handshake with {} sending public key", connection_tag);
        let salt = Salt::generate(SALT_LEN).map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
        let mut data = Vec::new();
        data.extend_from_slice(salt.as_ref());
        data.extend_from_slice(public_key.as_bytes());
        conn.send(&data)
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

        // Fifth, wait for a response that we will assume is the other side's salt & public key
        trace!(
            "Handshake with {} waiting for remote public key",
            connection_tag
        );
        let data = conn.next().await.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Stream ended before handshake completed",
            )
        })??;

        // If the data we received is too small, return an error
        if data.len() <= SALT_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Response had size smaller than expected",
            ));
        }

        let (salt_bytes, other_public_key_bytes) = data.split_at(SALT_LEN);
        let other_salt = Salt::from_slice(salt_bytes)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Sixth, decode the serialized public key from the other side
        let other_public_key = PublicKey::from_sec1_bytes(other_public_key_bytes)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Seventh, establish a shared secret that is NOT uniformly random, so we can't
        // directly use it as our encryption key (32 bytes in length)
        trace!("Handshake with {} computing shared secret", connection_tag);
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
        trace!("Handshake with {} deriving encryption key", connection_tag);
        let derived_key = kdf::derive_key(&password, &mixed_salt, 3, 1 << 16, 32)
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

        let crypt_key = Arc::new(derived_key);
        trace!("Finished handshake with {}", connection_tag);

        Ok(Self {
            conn,
            auth_key,
            crypt_key,
        })
    }

    /// Sends some data across the wire, waiting for it to completely send
    pub async fn send<D: Serialize>(&mut self, data: D) -> Result<(), TransportError> {
        send!(self.conn, self.crypt_key, self.auth_key.as_ref(), data).await
    }

    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    pub async fn receive<R: DeserializeOwned>(&mut self) -> Result<Option<R>, TransportError> {
        recv!(self.conn, self.crypt_key, self.auth_key).await
    }

    /// Returns a textual description of the transport's underlying connection
    pub fn to_connection_tag(&self) -> String {
        self.conn.get_ref().to_connection_tag()
    }

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
        send!(self.conn, self.crypt_key, self.auth_key.as_ref(), data).await
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
        recv!(self.conn, self.crypt_key, self.auth_key).await
    }
}

/// Test utilities
#[cfg(test)]
impl Transport<InmemoryStream> {
    /// Makes a connected pair of transports with matching crypt keys and using the provided
    /// auth keys
    pub fn make_pair_with_auth_keys(
        ak1: Option<Arc<SecretKey>>,
        ak2: Option<Arc<SecretKey>>,
    ) -> (Transport<InmemoryStream>, Transport<InmemoryStream>) {
        let crypt_key = Arc::new(SecretKey::default());

        let (a, b) = InmemoryStream::pair(crate::constants::test::BUFFER_SIZE);
        let a = Transport::new(a, ak1, Arc::clone(&crypt_key));
        let b = Transport::new(b, ak2, crypt_key);
        (a, b)
    }

    /// Makes a connected pair of transports with matching auth and crypt keys
    /// using test buffer size
    pub fn make_pair() -> (Transport<InmemoryStream>, Transport<InmemoryStream>) {
        Self::pair(crate::constants::test::BUFFER_SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::test::BUFFER_SIZE;
    use std::io;

    #[tokio::test]
    async fn transport_from_handshake_should_fail_if_connection_reached_eof() {
        // Cause nothing left incoming to stream by _
        let (_, mut rx, stream) = InmemoryStream::make(BUFFER_SIZE);
        let result = Transport::from_handshake(stream, None).await;

        // Verify that a salt and public key were sent out first
        // 1. Frame includes an 8 byte size at beginning
        // 2. Salt len + 256-bit (32 byte) public key + 1 byte tag (len) for pub key
        let outgoing = rx.recv().await.unwrap();
        assert_eq!(
            outgoing.len(),
            8 + SALT_LEN + 33,
            "Unexpected outgoing data: {:?}",
            outgoing
        );

        // Then confirm that failed because didn't receive anything back
        match result {
            Err(x) if x.kind() == io::ErrorKind::UnexpectedEof => {}
            Err(x) => panic!("Unexpected error: {:?}", x),
            Ok(_) => panic!("Unexpectedly succeeded!"),
        }
    }

    #[tokio::test]
    async fn transport_from_handshake_should_fail_if_response_data_is_too_small() {
        let (tx, _rx, stream) = InmemoryStream::make(BUFFER_SIZE);

        // Need SALT + PUB KEY where salt has a defined size; so, at least 1 larger than salt
        // would succeed, whereas we are providing exactly salt, which will fail
        {
            let mut frame = Vec::new();
            frame.extend_from_slice(&(SALT_LEN as u64).to_be_bytes());
            frame.extend_from_slice(Salt::generate(SALT_LEN).unwrap().as_ref());
            tx.send(frame).await.unwrap();
            drop(tx);
        }

        match Transport::from_handshake(stream, None).await {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {}
            Err(x) => panic!("Unexpected error: {:?}", x),
            Ok(_) => panic!("Unexpectedly succeeded!"),
        }
    }

    #[tokio::test]
    async fn transport_from_handshake_should_fail_if_bad_foreign_public_key_received() {
        let (tx, _rx, stream) = InmemoryStream::make(BUFFER_SIZE);

        // Send {SALT LEN}{SALT}{PUB KEY} where public key is bad;
        // normally public key bytes would be {LEN}{KEY} where len is first byte;
        // if the len does not match the rest of the message len, an error will be returned
        {
            let mut frame = Vec::new();
            frame.extend_from_slice(&((SALT_LEN + 3) as u64).to_be_bytes());
            frame.extend_from_slice(Salt::generate(SALT_LEN).unwrap().as_ref());
            frame.extend_from_slice(&[1, 1, 2]);
            tx.send(frame).await.unwrap();
            drop(tx);
        }

        match Transport::from_handshake(stream, None).await {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {
                let source = x.into_inner().expect("Inner source missing");
                assert_eq!(
                    source.to_string(),
                    "crypto error",
                    "Unexpected source: {}",
                    source
                );
            }
            Err(x) => panic!("Unexpected error: {:?}", x),
            Ok(_) => panic!("Unexpectedly succeeded!"),
        }
    }

    #[tokio::test]
    async fn transport_should_be_able_to_send_encrypted_data_to_other_side_to_decrypt() {
        // Make two transports with no auth keys
        let (mut src, mut dst) = Transport::make_pair_with_auth_keys(None, None);

        src.send("some data").await.expect("Failed to send data");
        let data = dst
            .receive::<String>()
            .await
            .expect("Failed to receive data")
            .expect("Data missing");

        assert_eq!(data, "some data");
    }

    #[tokio::test]
    async fn transport_should_be_able_to_sign_and_validate_signature_if_auth_key_included() {
        let auth_key = Arc::new(SecretKey::default());

        // Make two transports with same auth keys
        let (mut src, mut dst) =
            Transport::make_pair_with_auth_keys(Some(Arc::clone(&auth_key)), Some(auth_key));

        src.send("some data").await.expect("Failed to send data");
        let data = dst
            .receive::<String>()
            .await
            .expect("Failed to receive data")
            .expect("Data missing");

        assert_eq!(data, "some data");
    }

    #[tokio::test]
    async fn transport_receive_should_fail_if_auth_key_differs_from_other_end() {
        // Make two transports with different auth keys
        let (mut src, mut dst) = Transport::make_pair_with_auth_keys(
            Some(Arc::new(SecretKey::default())),
            Some(Arc::new(SecretKey::default())),
        );

        src.send("some data").await.expect("Failed to send data");
        match dst.receive::<String>().await {
            Err(TransportError::AuthError(_)) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn transport_receive_should_fail_if_has_auth_key_while_sender_did_not_use_one() {
        // Make two transports with different auth keys
        let (mut src, mut dst) =
            Transport::make_pair_with_auth_keys(None, Some(Arc::new(SecretKey::default())));

        src.send("some data").await.expect("Failed to send data");

        // NOTE: This keeps going between auth and io error about tag length because of the
        //       random data generated that can cause a different length to be perceived; so,
        //       we have to check for both
        match dst.receive::<String>().await {
            Err(TransportError::AuthError(_)) => {}
            Err(TransportError::IoError(x)) if matches!(x.kind(), io::ErrorKind::InvalidData) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn transport_receive_should_fail_if_has_no_auth_key_while_sender_used_one() {
        // Make two transports with different auth keys
        let (mut src, mut dst) =
            Transport::make_pair_with_auth_keys(Some(Arc::new(SecretKey::default())), None);

        src.send("some data").await.expect("Failed to send data");
        match dst.receive::<String>().await {
            Err(TransportError::EncryptError(_)) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }
}
