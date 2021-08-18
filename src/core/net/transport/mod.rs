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

    /// Sends some data across the wire, waiting for it to completely send
    #[allow(dead_code)]
    pub async fn send<D: Serialize>(&mut self, data: D) -> Result<(), TransportError> {
        send!(self.conn, self.crypt_key, self.auth_key.as_ref(), data).await
    }

    /// Receives some data from out on the wire, waiting until it's available,
    /// returning none if the transport is now closed
    #[allow(dead_code)]
    pub async fn receive<R: DeserializeOwned>(&mut self) -> Result<Option<R>, TransportError> {
        recv!(self.conn, self.crypt_key, self.auth_key).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io,
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::{io::ReadBuf, sync::mpsc};

    pub const TEST_DATA_STREAM_CHANNEL_BUFFER_SIZE: usize = 100;

    /// Represents a data stream comprised of two inmemory buffers of data
    pub struct TestDataStream {
        incoming: TestDataStreamReadHalf,
        outgoing: TestDataStreamWriteHalf,
    }

    impl TestDataStream {
        pub fn new(incoming: mpsc::Receiver<Vec<u8>>, outgoing: mpsc::Sender<Vec<u8>>) -> Self {
            Self {
                incoming: TestDataStreamReadHalf(incoming),
                outgoing: TestDataStreamWriteHalf(outgoing),
            }
        }

        /// Returns (incoming_tx, outgoing_rx, stream)
        pub fn make() -> (mpsc::Sender<Vec<u8>>, mpsc::Receiver<Vec<u8>>, Self) {
            let (incoming_tx, incoming_rx) = mpsc::channel(TEST_DATA_STREAM_CHANNEL_BUFFER_SIZE);
            let (outgoing_tx, outgoing_rx) = mpsc::channel(TEST_DATA_STREAM_CHANNEL_BUFFER_SIZE);

            (
                incoming_tx,
                outgoing_rx,
                Self::new(incoming_rx, outgoing_tx),
            )
        }

        /// Returns pair of streams that are connected such that one sends to the other and
        /// vice versa
        pub fn pair() -> (Self, Self) {
            let (tx, rx, stream) = Self::make();
            (stream, Self::new(rx, tx))
        }
    }

    impl AsyncRead for TestDataStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Pin::new(&mut self.incoming).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for TestDataStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.outgoing).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.outgoing).poll_flush(cx)
        }

        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.outgoing).poll_shutdown(cx)
        }
    }

    pub struct TestDataStreamReadHalf(mpsc::Receiver<Vec<u8>>);
    impl AsyncRead for TestDataStreamReadHalf {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            self.0.poll_recv(cx).map(|x| match x {
                Some(x) => {
                    buf.put_slice(&x);
                    Ok(())
                }
                None => Ok(()),
            })
        }
    }

    pub struct TestDataStreamWriteHalf(mpsc::Sender<Vec<u8>>);
    impl AsyncWrite for TestDataStreamWriteHalf {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            match self.0.try_send(buf.to_vec()) {
                Ok(_) => Poll::Ready(Ok(buf.len())),
                Err(_) => Poll::Ready(Ok(0)),
            }
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.poll_flush(cx)
        }
    }

    impl DataStream for TestDataStream {
        type Read = TestDataStreamReadHalf;
        type Write = TestDataStreamWriteHalf;

        fn to_connection_tag(&self) -> String {
            String::from("test-stream")
        }

        fn into_split(self) -> (Self::Read, Self::Write) {
            (self.incoming, self.outgoing)
        }
    }

    #[tokio::test]
    async fn transport_from_handshake_should_fail_if_connection_reached_eof() {
        // Cause nothing left incoming to stream by _
        let (_, mut rx, stream) = TestDataStream::make();
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
        let (tx, _rx, stream) = TestDataStream::make();

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
        let (tx, _rx, stream) = TestDataStream::make();

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
        let (src, dst) = TestDataStream::pair();

        // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
        let (src, dst) = tokio::join!(
            Transport::from_handshake(src, None),
            Transport::from_handshake(dst, None)
        );

        let mut src = src.expect("src stream failed handshake");
        let mut dst = dst.expect("dst stream failed handshake");

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
        let (src, dst) = TestDataStream::pair();

        let auth_key = Arc::new(SecretKey::default());

        // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
        let (src, dst) = tokio::join!(
            Transport::from_handshake(src, Some(Arc::clone(&auth_key))),
            Transport::from_handshake(dst, Some(auth_key))
        );

        let mut src = src.expect("src stream failed handshake");
        let mut dst = dst.expect("dst stream failed handshake");

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
        let (src, dst) = TestDataStream::pair();

        // Make two transports with different auth keys
        // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
        let (src, dst) = tokio::join!(
            Transport::from_handshake(src, Some(Arc::new(SecretKey::default()))),
            Transport::from_handshake(dst, Some(Arc::new(SecretKey::default())))
        );

        let mut src = src.expect("src stream failed handshake");
        let mut dst = dst.expect("dst stream failed handshake");

        src.send("some data").await.expect("Failed to send data");
        match dst.receive::<String>().await {
            Err(TransportError::AuthError(_)) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn transport_receive_should_fail_if_has_auth_key_while_sender_did_not_use_one() {
        let (src, dst) = TestDataStream::pair();

        // Make two transports with different auth keys
        // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
        let (src, dst) = tokio::join!(
            Transport::from_handshake(dst, None),
            Transport::from_handshake(src, Some(Arc::new(SecretKey::default())))
        );

        let mut src = src.expect("src stream failed handshake");
        let mut dst = dst.expect("dst stream failed handshake");

        src.send("some data").await.expect("Failed to send data");
        match dst.receive::<String>().await {
            Err(TransportError::AuthError(_)) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn transport_receive_should_fail_if_has_no_auth_key_while_sender_used_one() {
        let (src, dst) = TestDataStream::pair();

        // Make two transports with different auth keys
        // NOTE: This is slow during tests as it is an expensive process and we're doing it twice!
        let (src, dst) = tokio::join!(
            Transport::from_handshake(src, Some(Arc::new(SecretKey::default()))),
            Transport::from_handshake(dst, None)
        );

        let mut src = src.expect("src stream failed handshake");
        let mut dst = dst.expect("dst stream failed handshake");

        src.send("some data").await.expect("Failed to send data");
        match dst.receive::<String>().await {
            Err(TransportError::EncryptError(_)) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }
}
