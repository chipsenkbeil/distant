use std::future::Future;
use std::io;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;

use crate::auth::{AuthHandler, Authenticate, Verifier};
use log::*;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

#[cfg(test)]
use crate::net::common::InmemoryTransport;
use crate::net::common::{
    Backup, FramedTransport, HeapSecretKey, Keychain, KeychainResult, Reconnectable, Transport,
    TransportExt, Version,
};

/// Id of the connection
pub type ConnectionId = u32;

/// Represents a connection from either the client or server side
#[derive(Debug)]
pub enum Connection<T> {
    /// Connection from the client side
    Client {
        /// Unique id associated with the connection
        id: ConnectionId,

        /// One-time password (OTP) for use in reauthenticating with the server
        reauth_otp: HeapSecretKey,

        /// Underlying transport used to communicate
        transport: FramedTransport<T>,
    },

    /// Connection from the server side
    Server {
        /// Unique id associated with the connection
        id: ConnectionId,

        /// Used to send the backup into storage when the connection is dropped
        tx: oneshot::Sender<Backup>,

        /// Underlying transport used to communicate
        transport: FramedTransport<T>,
    },
}

impl<T> Deref for Connection<T> {
    type Target = FramedTransport<T>;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Client { transport, .. } => transport,
            Self::Server { transport, .. } => transport,
        }
    }
}

impl<T> DerefMut for Connection<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Client { transport, .. } => transport,
            Self::Server { transport, .. } => transport,
        }
    }
}

impl<T> Drop for Connection<T> {
    /// On drop for a server connection, the connection's backup will be sent via `tx`. For a
    /// client connection, nothing happens.
    fn drop(&mut self) {
        match self {
            Self::Client { .. } => (),
            Self::Server { tx, transport, .. } => {
                // NOTE: We grab the current backup state and store it using the tx, replacing
                //       the backup with a default and the tx with a disconnected one
                let backup = std::mem::take(&mut transport.backup);
                let tx = std::mem::replace(tx, oneshot::channel().0);
                let _ = tx.send(backup);
            }
        }
    }
}

impl<T> Reconnectable for Connection<T>
where
    T: Transport,
{
    /// Attempts to re-establish a connection.
    ///
    /// ### Client
    ///
    /// For a client, this means performing an actual [`reconnect`] on the underlying
    /// [`Transport`], re-establishing an encrypted codec, submitting a request to the server to
    /// reauthenticate using a previously-derived OTP, and refreshing the  OTP for use in a future
    /// reauthentication.
    ///
    /// ### Server
    ///
    /// For a server, this will fail as unsupported.
    ///
    /// [`reconnect`]: Reconnectable::reconnect
    fn reconnect<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            async fn reconnect_client<T: Transport>(
                id: ConnectionId,
                reauth_otp: HeapSecretKey,
                transport: &mut FramedTransport<T>,
            ) -> io::Result<(ConnectionId, HeapSecretKey)> {
                // Re-establish a raw connection
                debug!("[Conn {id}] Re-establishing connection");
                Reconnectable::reconnect(transport).await?;

                // Wait for exactly version bytes (24 where 8 bytes for major, minor, patch)
                // but with a reconnect we don't actually validate it because we did that
                // the first time we connected
                //
                // NOTE: We do this with the raw transport and not the framed version!
                debug!("[Conn {id}] Waiting for server version");
                if transport.as_mut_inner().read_exact(&mut [0u8; 24]).await? != 24 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Wrong version byte len received",
                    ));
                }

                // Perform a handshake to ensure that the connection is properly established and encrypted
                debug!("[Conn {id}] Performing handshake");
                transport.client_handshake().await?;

                // Communicate that we are an existing connection
                debug!("[Conn {id}] Performing re-authentication");
                transport
                    .write_frame_for(&ConnectType::Reconnect {
                        id,
                        otp: reauth_otp.unprotected_into_bytes(),
                    })
                    .await?;

                // Derive an OTP for reauthentication
                debug!("[Conn {id}] Deriving future OTP for reauthentication");
                let reauth_otp = transport.exchange_keys().await?.into_heap_secret_key();

                Ok((id, reauth_otp))
            }

            match self {
                Self::Client {
                    id,
                    transport,
                    reauth_otp,
                } => {
                    // Freeze our backup as we don't want the connection logic to alter it, attempt to
                    // perform the reconnection, and unfreeze our backup regardless of the result
                    let (new_id, new_reauth_otp) = {
                        transport.backup.freeze();
                        let result = reconnect_client(*id, reauth_otp.clone(), transport).await;
                        transport.backup.unfreeze();
                        result?
                    };

                    // Perform synchronization
                    debug!("[Conn {id}] Synchronizing frame state");
                    transport.synchronize().await?;

                    // Everything has succeeded, so we now will update our id and reauth otp
                    info!(
                        "[Conn {id}] Reconnect completed successfully! Assigning new id {new_id}"
                    );
                    *id = new_id;
                    *reauth_otp = new_reauth_otp;

                    Ok(())
                }

                Self::Server { .. } => Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Server connection cannot reconnect",
                )),
            }
        })
    }
}

/// Type of connection to perform
#[derive(Debug, Serialize, Deserialize)]
enum ConnectType {
    /// Indicates that the connection from client to server is no and not a reconnection
    Connect,

    /// Indicates that the connection from client to server is a reconnection and should attempt to
    /// use the connection id and OTP to authenticate
    Reconnect {
        /// Id of the connection to reauthenticate
        id: ConnectionId,

        /// Raw bytes of the OTP
        #[serde(with = "serde_bytes")]
        otp: Vec<u8>,
    },
}

impl<T> Connection<T>
where
    T: Transport,
{
    /// Transforms a raw [`Transport`] into an established [`Connection`] from the client-side by
    /// performing the following:
    ///
    /// 1. Performs a version check with the server
    /// 2. Handshakes to derive the appropriate [`Codec`](crate::Codec) to use
    /// 3. Authenticates the established connection to ensure it is valid
    /// 4. Restores pre-existing state using the provided backup, replaying any missing frames and
    ///    receiving any frames from the other side
    pub async fn client<H: AuthHandler + Send>(
        transport: T,
        handler: H,
        version: Version,
    ) -> io::Result<Self> {
        let id: ConnectionId = rand::random();

        // Wait for exactly version bytes (24 where 8 bytes for major, minor, patch)
        debug!("[Conn {id}] Waiting for server version");
        let mut version_bytes = [0u8; 24];
        if transport.read_exact(&mut version_bytes).await? != 24 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Wrong version byte len received",
            ));
        }

        // Compare versions for compatibility and drop the connection if incompatible
        let server_version = Version::from_be_bytes(version_bytes);
        debug!(
            "[Conn {id}] Checking compatibility between client {version} & server {server_version}"
        );
        if !version.is_compatible_with(&server_version) {
            return Err(io::Error::other(format!(
                "Client version {version} is incompatible with server version {server_version}"
            )));
        }

        // Perform a handshake to ensure that the connection is properly established and encrypted
        debug!("[Conn {id}] Performing handshake");
        let mut transport: FramedTransport<T> =
            FramedTransport::from_client_handshake(transport).await?;

        // Communicate that we are a new connection
        debug!("[Conn {id}] Communicating that this is a new connection");
        transport.write_frame_for(&ConnectType::Connect).await?;

        // Receive the new id for the connection
        let id = {
            debug!("[Conn {id}] Receiving new connection id");
            let new_id = transport
                .read_frame_as::<ConnectionId>()
                .await?
                .ok_or_else(|| io::Error::other("Missing connection id frame"))?;
            debug!("[Conn {id}] Resetting id to {new_id}");
            new_id
        };

        // Authenticate the transport with the server-side
        debug!("[Conn {id}] Performing authentication");
        transport.authenticate(handler).await?;

        // Derive an OTP for reauthentication
        debug!("[Conn {id}] Deriving future OTP for reauthentication");
        let reauth_otp = transport.exchange_keys().await?.into_heap_secret_key();

        info!("[Conn {id}] Connect completed successfully!");
        Ok(Self::Client {
            id,
            reauth_otp,
            transport,
        })
    }

    /// Transforms a raw [`Transport`] into an established [`Connection`] from the server-side by
    /// performing the following:
    ///
    /// 1. Performs a version check with the client
    /// 2. Handshakes to derive the appropriate [`Codec`](crate::Codec) to use
    /// 3. Authenticates the established connection to ensure it is valid by either using the
    ///    given `verifier` or, if working with an existing client connection, will validate an OTP
    ///    from our database
    /// 4. Restores pre-existing state using the provided backup, replaying any missing frames and
    ///    receiving any frames from the other side
    pub async fn server(
        transport: T,
        verifier: &Verifier,
        keychain: Keychain<oneshot::Receiver<Backup>>,
        version: Version,
    ) -> io::Result<Self> {
        let id: ConnectionId = rand::random();

        // Write the version as bytes
        debug!("[Conn {id}] Sending version {version}");
        transport.write_all(&version.to_be_bytes()).await?;

        // Perform a handshake to ensure that the connection is properly established and encrypted
        debug!("[Conn {id}] Performing handshake");
        let mut transport: FramedTransport<T> =
            FramedTransport::from_server_handshake(transport).await?;

        // Receive a client id, look up to see if the client id exists already
        //
        // 1. If it already exists, wait for a password to follow, which is a one-time password used by
        //    the client. If the password is correct, then generate a new one-time client id and
        //    password for a future connection (only updating if the connection fully completes) and
        //    send it to the client, and then perform a replay situation
        //
        // 2. If it does not exist, ignore the client id and password. Generate a new client id to send
        //    to the client. Perform verification like usual. Then generate a one-time password and
        //    send it to the client.
        debug!("[Conn {id}] Waiting for connection type");
        let connection_type = transport
            .read_frame_as::<ConnectType>()
            .await?
            .ok_or_else(|| io::Error::other("Missing connection type frame"))?;

        // Create a oneshot channel used to relay the backup when the connection is dropped
        let (tx, rx) = oneshot::channel();

        // Based on the connection type, we either try to find and validate an existing connection
        // or we perform normal verification
        let id = match connection_type {
            ConnectType::Connect => {
                // Communicate the connection id
                debug!("[Conn {id}] Telling other side to change connection id");
                transport.write_frame_for(&id).await?;

                // Perform authentication to ensure the connection is valid
                debug!("[Conn {id}] Verifying connection");
                verifier.verify(&mut transport).await?;

                // Derive an OTP for reauthentication
                debug!("[Conn {id}] Deriving future OTP for reauthentication");
                let reauth_otp = transport.exchange_keys().await?.into_heap_secret_key();

                // Store the id, OTP, and backup retrieval in our database
                info!("[Conn {id}] Connect completed successfully!");
                keychain.insert(id.to_string(), reauth_otp, rx).await;

                id
            }
            ConnectType::Reconnect { id: other_id, otp } => {
                let reauth_otp = HeapSecretKey::from(otp);

                debug!("[Conn {id}] Checking if {other_id} exists and has matching OTP");
                match keychain
                    .remove_if_has_key(other_id.to_string(), reauth_otp.clone())
                    .await
                {
                    KeychainResult::Ok(x) => {
                        // Match found, so we want ot update our id to be the pre-existing id
                        debug!("[Conn {id}] Reassigning to {other_id}");
                        let id = other_id;

                        // Grab the old backup
                        debug!("[Conn {id}] Acquiring backup for existing connection");
                        let backup = match x.await {
                            Ok(backup) => backup,
                            Err(_) => {
                                warn!("[Conn {id}] Missing backup, will use fresh copy");
                                Backup::new()
                            }
                        };

                        macro_rules! unwrap_or_fail {
                            ($action:expr) => {
                                unwrap_or_fail!(backup, $action)
                            };
                            ($backup:expr, $action:expr) => {{
                                match $action {
                                    Ok(x) => x,
                                    Err(x) => {
                                        error!("[Conn {id}] Encountered error, restoring with old backup");
                                        let _ = tx.send($backup);
                                        keychain.insert(id.to_string(), reauth_otp, rx).await;
                                        return Err(x);
                                    }
                                }
                            }};
                        }

                        // Derive an OTP for reauthentication
                        debug!("[Conn {id}] Deriving future OTP for reauthentication");
                        let new_reauth_otp =
                            unwrap_or_fail!(transport.exchange_keys().await).into_heap_secret_key();

                        // Replace our backup with the old one
                        debug!("[Conn {id}] Restoring backup");
                        transport.backup = backup;

                        // Synchronize using the provided backup
                        debug!("[Conn {id}] Synchronizing frame state");
                        unwrap_or_fail!(transport.backup, transport.synchronize().await);

                        // Store the id, OTP, and backup retrieval in our database
                        info!("[Conn {id}] Reconnect restoration completed successfully!");
                        keychain.insert(id.to_string(), new_reauth_otp, rx).await;

                        id
                    }
                    KeychainResult::InvalidPassword => {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "Invalid OTP for reconnect",
                        ));
                    }
                    KeychainResult::InvalidId => {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "Invalid id for reconnect",
                        ));
                    }
                }
            }
        };

        Ok(Self::Server { id, tx, transport })
    }
}

#[cfg(test)]
impl Connection<InmemoryTransport> {
    /// Establishes a pair of [`Connection`]s using [`InmemoryTransport`] underneath, returning
    /// them in the form (client, server).
    ///
    /// ### Note
    ///
    /// This skips handshakes, authentication, and backup processing. These connections cannot be
    /// reconnected and have no encryption.
    pub fn pair(buffer: usize) -> (Self, Self) {
        let id = rand::random::<ConnectionId>();
        let (t1, t2) = FramedTransport::pair(buffer);

        let client = Connection::Client {
            id,
            reauth_otp: HeapSecretKey::generate(32).unwrap(),
            transport: t1,
        };

        let server = Connection::Server {
            id,
            tx: oneshot::channel().0,
            transport: t2,
        };

        (client, server)
    }
}

impl<T> Connection<T> {
    /// Returns the id of the connection.
    pub fn id(&self) -> ConnectionId {
        match self {
            Self::Client { id, .. } => *id,
            Self::Server { id, .. } => *id,
        }
    }
}

#[cfg(test)]
impl<T> Connection<T> {
    /// Returns the OTP associated with the connection, or none if connection is server-side.
    pub fn otp(&self) -> Option<&HeapSecretKey> {
        match self {
            Self::Client { reauth_otp, .. } => Some(reauth_otp),
            Self::Server { .. } => None,
        }
    }

    /// Returns a reference to the underlying transport.
    pub fn transport(&self) -> &FramedTransport<T> {
        match self {
            Self::Client { transport, .. } => transport,
            Self::Server { transport, .. } => transport,
        }
    }

    /// Returns a mutable reference to the underlying transport.
    pub fn mut_transport(&mut self) -> &mut FramedTransport<T> {
        match self {
            Self::Client { transport, .. } => transport,
            Self::Server { transport, .. } => transport,
        }
    }
}

#[cfg(test)]
impl<T: Transport> Connection<T> {
    pub fn test_client(transport: T) -> Self {
        Self::Client {
            id: rand::random(),
            reauth_otp: HeapSecretKey::generate(32).unwrap(),
            transport: FramedTransport::plain(transport),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::auth::msg::Challenge;
    use crate::auth::{Authenticator, DummyAuthHandler};
    use test_log::test;

    use super::*;
    use crate::net::common::Frame;

    macro_rules! server_version {
        () => {
            Version::new(1, 2, 3)
        };
    }

    macro_rules! send_server_version {
        ($transport:expr, $version:expr) => {{
            ($transport)
                .as_mut_inner()
                .write_all(&$version.to_be_bytes())
                .await
                .unwrap();
        }};
        ($transport:expr) => {
            send_server_version!($transport, server_version!());
        };
    }

    macro_rules! receive_version {
        ($transport:expr) => {{
            let mut bytes = [0u8; 24];
            assert_eq!(
                ($transport)
                    .as_mut_inner()
                    .read_exact(&mut bytes)
                    .await
                    .unwrap(),
                24,
                "Wrong version len received"
            );
            Version::from_be_bytes(bytes)
        }};
    }

    #[test(tokio::test)]
    async fn client_should_fail_when_server_sends_incompatible_version() {
        let (mut t1, t2) = FramedTransport::pair(100);

        // Spawn a task to perform the client connection so we don't deadlock while simulating the
        // server actions on the other side
        let task = tokio::spawn(async move {
            Connection::client(t2.into_inner(), DummyAuthHandler, Version::new(1, 2, 3))
                .await
                .unwrap()
        });

        // Send invalid version to fail the handshake
        send_server_version!(t1, Version::new(2, 0, 0));

        // Client should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn client_should_fail_if_codec_handshake_fails() {
        let (mut t1, t2) = FramedTransport::pair(100);

        // Spawn a task to perform the client connection so we don't deadlock while simulating the
        // server actions on the other side
        let task = tokio::spawn(async move {
            Connection::client(t2.into_inner(), DummyAuthHandler, server_version!())
                .await
                .unwrap()
        });

        // Send server version for client to confirm
        send_server_version!(t1);

        // Send garbage to fail the handshake
        t1.write_frame(Frame::new(b"invalid")).await.unwrap();

        // Client should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn client_should_fail_if_unable_to_receive_connection_id_from_server() {
        let (mut t1, t2) = FramedTransport::pair(100);

        // Spawn a task to perform the client connection so we don't deadlock while simulating the
        // server actions on the other side
        let task = tokio::spawn(async move {
            Connection::client(t2.into_inner(), DummyAuthHandler, server_version!())
                .await
                .unwrap()
        });

        // Send server version for client to confirm
        send_server_version!(t1);

        // Perform first step of connection by establishing the codec
        t1.server_handshake().await.unwrap();

        // Receive a type that indicates a new connection
        let ct = t1.read_frame_as::<ConnectType>().await.unwrap().unwrap();
        assert!(
            matches!(ct, ConnectType::Connect),
            "Unexpected connect type: {ct:?}"
        );

        // Drop to cause id retrieval on client to fail
        drop(t1);

        // Client should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn client_should_fail_if_authentication_fails() {
        let (mut t1, t2) = FramedTransport::pair(100);

        // Spawn a task to perform the client connection so we don't deadlock while simulating the
        // server actions on the other side
        let task = tokio::spawn(async move {
            Connection::client(t2.into_inner(), DummyAuthHandler, server_version!())
                .await
                .unwrap()
        });

        // Send server version for client to confirm
        send_server_version!(t1);

        // Perform first step of connection by establishing the codec
        t1.server_handshake().await.unwrap();

        // Receive a type that indicates a new connection
        let ct = t1.read_frame_as::<ConnectType>().await.unwrap().unwrap();
        assert!(
            matches!(ct, ConnectType::Connect),
            "Unexpected connect type: {ct:?}"
        );

        // Send a connection id as second step of connection
        t1.write_frame_for(&rand::random::<ConnectionId>())
            .await
            .unwrap();

        // Perform an authentication request that will fail on the client side, which will
        // cause the client to drop and therefore this transport to fail in getting a response
        t1.challenge(Challenge {
            questions: Vec::new(),
            options: Default::default(),
        })
        .await
        .unwrap_err();

        // Client should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn client_should_fail_if_unable_to_exchange_otp_for_reauthentication() {
        let (mut t1, t2) = FramedTransport::pair(100);

        // Spawn a task to perform the client connection so we don't deadlock while simulating the
        // server actions on the other side
        let task = tokio::spawn(async move {
            Connection::client(t2.into_inner(), DummyAuthHandler, server_version!())
                .await
                .unwrap()
        });

        // Send server version for client to confirm
        send_server_version!(t1);

        // Perform first step of connection by establishing the codec
        t1.server_handshake().await.unwrap();

        // Receive a type that indicates a new connection
        let ct = t1.read_frame_as::<ConnectType>().await.unwrap().unwrap();
        assert!(
            matches!(ct, ConnectType::Connect),
            "Unexpected connect type: {ct:?}"
        );

        // Send a connection id as second step of connection
        t1.write_frame_for(&rand::random::<ConnectionId>())
            .await
            .unwrap();

        // Perform verification as third step using none method, which should always succeed
        // without challenging
        Verifier::none().verify(&mut t1).await.unwrap();

        // Send garbage to fail the key exchange
        t1.write_frame(Frame::new(b"invalid")).await.unwrap();

        // Client should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn client_should_succeed_if_establishes_connection_with_server() {
        let (mut t1, t2) = FramedTransport::pair(100);

        // Spawn a task to perform the client connection so we don't deadlock while simulating the
        // server actions on the other side
        let task = tokio::spawn(async move {
            Connection::client(t2.into_inner(), DummyAuthHandler, server_version!())
                .await
                .unwrap()
        });

        // Send server version for client to confirm
        send_server_version!(t1);

        // Perform first step of connection by establishing the codec
        t1.server_handshake().await.unwrap();

        // Receive a type that indicates a new connection
        let ct = t1.read_frame_as::<ConnectType>().await.unwrap().unwrap();
        assert!(
            matches!(ct, ConnectType::Connect),
            "Unexpected connect type: {ct:?}"
        );

        // Send a connection id as second step of connection
        t1.write_frame_for(&rand::random::<ConnectionId>())
            .await
            .unwrap();

        // Perform verification as third step using none method, which should always succeed
        // without challenging
        Verifier::none().verify(&mut t1).await.unwrap();

        // Perform fourth step of key exchange for OTP
        let otp = t1.exchange_keys().await.unwrap().into_heap_secret_key();

        // Client should succeed and have an OTP that matches the server-side version
        let client = task.await.unwrap();
        assert_eq!(client.otp(), Some(&otp));
    }

    #[test(tokio::test)]
    async fn server_should_fail_if_client_drops_due_to_version() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn(async move {
            Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                .await
                .unwrap()
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Drop client connection as a result of an "incompatible version"
        drop(t1);

        // Server should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn server_should_fail_if_codec_handshake_fails() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn(async move {
            Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                .await
                .unwrap()
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Send garbage to fail the handshake
        t1.write_frame(Frame::new(b"invalid")).await.unwrap();

        // Server should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn server_should_fail_if_unable_to_receive_connect_type() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn(async move {
            Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                .await
                .unwrap()
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Perform first step of completing client-side of handshake
        t1.client_handshake().await.unwrap();

        // Send some garbage that is not the connection type
        t1.write_frame(Frame::new(b"hello")).await.unwrap();

        // Server should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn server_should_fail_if_unable_to_verify_new_client() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::static_key(HeapSecretKey::generate(32).unwrap());
        let keychain = Keychain::new();

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn(async move {
            Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                .await
                .unwrap()
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Perform first step of completing client-side of handshake
        t1.client_handshake().await.unwrap();

        // Send type to indicate a new connection
        t1.write_frame_for(&ConnectType::Connect).await.unwrap();

        // Receive the connection id
        let _id = t1.read_frame_as::<ConnectionId>().await.unwrap().unwrap();

        // Fail verification using the dummy handler that will fail when asked for a static key
        t1.authenticate(DummyAuthHandler).await.unwrap_err();

        // Drop the transport so we kill the server-side connection
        // NOTE: If we don't drop here, the above authentication failure won't kill the server
        drop(t1);

        // Server should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn server_should_fail_if_unable_to_exchange_otp_for_reauthentication_with_new_client() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn(async move {
            Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                .await
                .unwrap()
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Perform first step of completing client-side of handshake
        t1.client_handshake().await.unwrap();

        // Send type to indicate a new connection
        t1.write_frame_for(&ConnectType::Connect).await.unwrap();

        // Receive the connection id
        let _id = t1.read_frame_as::<ConnectionId>().await.unwrap().unwrap();

        // Pass verification using the dummy handler since our verifier supports no authentication
        t1.authenticate(DummyAuthHandler).await.unwrap();

        // Send some garbage to fail the exchange
        t1.write_frame(Frame::new(b"hello")).await.unwrap();

        // Server should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn server_should_fail_if_existing_client_id_is_invalid() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn(async move {
            Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                .await
                .unwrap()
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Perform first step of completing client-side of handshake
        t1.client_handshake().await.unwrap();

        // Send type to indicate an existing connection, which should cause the server-side to fail
        // because there is no matching id
        t1.write_frame_for(&ConnectType::Reconnect {
            id: 1234,
            otp: HeapSecretKey::generate(32)
                .unwrap()
                .unprotected_into_bytes(),
        })
        .await
        .unwrap();

        // Server should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn server_should_fail_if_existing_client_otp_is_invalid() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();

        keychain
            .insert(
                1234.to_string(),
                HeapSecretKey::generate(32).unwrap(),
                oneshot::channel().1,
            )
            .await;

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn(async move {
            Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                .await
                .unwrap()
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Perform first step of completing client-side of handshake
        t1.client_handshake().await.unwrap();

        // Send type to indicate an existing connection, which should cause the server-side to fail
        // because the OTP is wrong for the given id
        t1.write_frame_for(&ConnectType::Reconnect {
            id: 1234,
            otp: HeapSecretKey::generate(32)
                .unwrap()
                .unprotected_into_bytes(),
        })
        .await
        .unwrap();

        // Server should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn server_should_fail_if_unable_to_exchange_otp_for_reauthentication_with_existing_client()
     {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();
        let key = HeapSecretKey::generate(32).unwrap();

        keychain
            .insert(1234.to_string(), key.clone(), oneshot::channel().1)
            .await;

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn(async move {
            Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                .await
                .unwrap()
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Perform first step of completing client-side of handshake
        t1.client_handshake().await.unwrap();

        // Send type to indicate an existing connection, which should cause the server-side to fail
        // because the OTP is wrong for the given id
        t1.write_frame_for(&ConnectType::Reconnect {
            id: 1234,
            otp: key.unprotected_into_bytes(),
        })
        .await
        .unwrap();

        // Send garbage to fail the otp exchange
        t1.write_frame(Frame::new(b"hello")).await.unwrap();

        // Server should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn server_should_fail_if_unable_to_synchronize_with_existing_client() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();
        let key = HeapSecretKey::generate(32).unwrap();

        keychain
            .insert(1234.to_string(), key.clone(), oneshot::channel().1)
            .await;

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn(async move {
            Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                .await
                .unwrap()
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Perform first step of completing client-side of handshake
        t1.client_handshake().await.unwrap();

        // Send type to indicate an existing connection, which should cause the server-side to fail
        // because the OTP is wrong for the given id
        t1.write_frame_for(&ConnectType::Reconnect {
            id: 1234,
            otp: key.unprotected_into_bytes(),
        })
        .await
        .unwrap();

        // Perform otp exchange
        let _otp = t1.exchange_keys().await.unwrap();

        // Send garbage to fail synchronization
        t1.write_frame(b"hello").await.unwrap();

        // Server should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn server_should_succeed_if_establishes_connection_with_new_client() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn({
            let keychain = keychain.clone();
            async move {
                Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                    .await
                    .unwrap()
            }
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Perform first step of completing client-side of handshake
        t1.client_handshake().await.unwrap();

        // Send type to indicate a new connection
        t1.write_frame_for(&ConnectType::Connect).await.unwrap();

        // Receive the connection id
        let id = t1.read_frame_as::<ConnectionId>().await.unwrap().unwrap();

        // Pass verification using the dummy handler since our verifier supports no authentication
        t1.authenticate(DummyAuthHandler).await.unwrap();

        // Perform otp exchange
        let otp = t1.exchange_keys().await.unwrap();

        // Server connection should be established, and have received some replayed frames
        let server = task.await.unwrap();

        // Validate the connection ids match
        assert_eq!(server.id(), id);

        // Validate the OTP was stored in our keychain
        assert!(
            keychain
                .has_key(id.to_string(), otp.into_heap_secret_key())
                .await,
            "Missing OTP"
        );
    }

    #[test(tokio::test)]
    async fn server_should_succeed_if_establishes_connection_with_existing_client() {
        let (mut t1, t2) = FramedTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();
        let key = HeapSecretKey::generate(32).unwrap();
        let id = 1234;

        keychain
            .insert(id.to_string(), key.clone(), {
                // Create a custom backup we'll use to replay frames from the server-side
                let mut backup = Backup::new();

                backup.push_frame(Frame::new(b"hello"));
                backup.push_frame(Frame::new(b"world"));
                backup.increment_sent_cnt();
                backup.increment_sent_cnt();

                let (tx, rx) = oneshot::channel();
                tx.send(backup).unwrap();
                rx
            })
            .await;

        // Spawn a task to perform the server connection so we don't deadlock while simulating the
        // client actions on the other side
        let task = tokio::spawn({
            let keychain = keychain.clone();
            async move {
                Connection::server(t2.into_inner(), &verifier, keychain, server_version!())
                    .await
                    .unwrap()
            }
        });

        // Receive the version from the server
        let _ = receive_version!(t1);

        // Perform first step of completing client-side of handshake
        t1.client_handshake().await.unwrap();

        // Send type to indicate an existing connection, which should cause the server-side to fail
        // because the OTP is wrong for the given id
        t1.write_frame_for(&ConnectType::Reconnect {
            id: 1234,
            otp: key.unprotected_into_bytes(),
        })
        .await
        .unwrap();

        // Perform otp exchange
        let otp = t1.exchange_keys().await.unwrap();

        // Queue up some frames to send to the server
        t1.backup.clear();
        t1.backup.push_frame(Frame::new(b"foo"));
        t1.backup.push_frame(Frame::new(b"bar"));
        t1.backup.increment_sent_cnt();
        t1.backup.increment_sent_cnt();

        // Perform synchronization
        t1.synchronize().await.unwrap();

        // Verify that we received frames from the server
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"hello");
        assert_eq!(t1.read_frame().await.unwrap().unwrap(), b"world");

        // Server connection should be established, and have received some replayed frames
        let mut server = task.await.unwrap();
        assert_eq!(server.read_frame().await.unwrap().unwrap(), b"foo");
        assert_eq!(server.read_frame().await.unwrap().unwrap(), b"bar");

        // Validate the connection ids match
        assert_eq!(server.id(), id);

        // Validate the OTP was stored in our keychain
        assert!(
            keychain
                .has_key(id.to_string(), otp.into_heap_secret_key())
                .await,
            "Missing OTP"
        );
    }

    #[test(tokio::test)]
    async fn client_server_new_connection_e2e_should_establish_connection() {
        let (t1, t2) = InmemoryTransport::pair(100);
        let verifier = Verifier::none();
        let keychain = Keychain::new();

        // Spawn a task to perform the server connection so we don't deadlock
        let task = tokio::spawn(async move {
            Connection::server(t2, &verifier, keychain, server_version!())
                .await
                .expect("Failed to connect from server")
        });

        // Perform the client-side of the connection
        let mut client = Connection::client(t1, DummyAuthHandler, server_version!())
            .await
            .expect("Failed to connect from client");
        let mut server = task.await.unwrap();

        // Test out the connection
        client.write_frame(Frame::new(b"hello")).await.unwrap();
        assert_eq!(server.read_frame().await.unwrap().unwrap(), b"hello");
        server.write_frame(Frame::new(b"goodbye")).await.unwrap();
        assert_eq!(client.read_frame().await.unwrap().unwrap(), b"goodbye");
    }

    /// Helper utility to set up for a client reconnection
    async fn setup_reconnect_scenario() -> (
        Connection<InmemoryTransport>,
        InmemoryTransport,
        Arc<Verifier>,
        Keychain<oneshot::Receiver<Backup>>,
    ) {
        let (t1, t2) = InmemoryTransport::pair(100);
        let verifier = Arc::new(Verifier::none());
        let keychain = Keychain::new();

        // Spawn a task to perform the server connection so we don't deadlock
        let task = {
            let verifier = Arc::clone(&verifier);
            let keychain = keychain.clone();
            tokio::spawn(async move {
                Connection::server(t2, &verifier, keychain, server_version!())
                    .await
                    .expect("Failed to connect from server")
            })
        };

        // Perform the client-side of the connection
        let mut client = Connection::client(t1, DummyAuthHandler, server_version!())
            .await
            .expect("Failed to connect from client");

        // Ensure the server is established and then drop it
        let server = task.await.unwrap();
        drop(server);

        // Create a new inmemory transport and link it to the client
        let mut t2 = InmemoryTransport::pair(100).0;
        t2.link(client.mut_transport().as_mut_inner(), 100);

        (client, t2, verifier, keychain)
    }

    #[test(tokio::test)]
    async fn reconnect_should_fail_if_client_side_connection_handshake_fails() {
        let (mut client, transport, _verifier, _keychain) = setup_reconnect_scenario().await;
        let mut transport = FramedTransport::plain(transport);

        // Spawn a task to perform the client reconnection so we don't deadlock
        let task = tokio::spawn(async move { client.reconnect().await.unwrap() });

        // Send a version, although it'll be ignored by a reconnecting client
        send_server_version!(transport);

        // Send garbage to fail handshake from server-side
        transport.write_frame(b"hello").await.unwrap();

        // Client should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn reconnect_should_fail_if_client_side_connection_unable_to_receive_new_connection_id() {
        let (mut client, transport, _verifier, _keychain) = setup_reconnect_scenario().await;
        let mut transport = FramedTransport::plain(transport);

        // Spawn a task to perform the client reconnection so we don't deadlock
        let task = tokio::spawn(async move { client.reconnect().await.unwrap() });

        // Send a version, although it'll be ignored by a reconnecting client
        send_server_version!(transport);

        // Perform first step of completing server-side of handshake
        transport.server_handshake().await.unwrap();

        // Drop transport to cause client to fail in not receiving connection id
        drop(transport);

        // Client should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn reconnect_should_fail_if_client_side_connection_unable_to_exchange_otp_with_server() {
        let (mut client, transport, _verifier, keychain) = setup_reconnect_scenario().await;
        let mut transport = FramedTransport::plain(transport);

        // Spawn a task to perform the client reconnection so we don't deadlock
        let task = tokio::spawn(async move { client.reconnect().await.unwrap() });

        // Send a version, although it'll be ignored by a reconnecting client
        send_server_version!(transport);

        // Perform first step of completing server-side of handshake
        transport.server_handshake().await.unwrap();

        // Receive reconnect data from client-side
        let (id, otp) = match transport.read_frame_as::<ConnectType>().await {
            Ok(Some(ConnectType::Reconnect { id, otp })) => (id, HeapSecretKey::from(otp)),
            x => panic!("Unexpected result: {x:?}"),
        };

        // Verify the id and OTP matches the one stored into our keychain from the setup
        assert!(
            keychain.has_key(id.to_string(), otp).await,
            "Wrong id or OTP"
        );

        // Send a new id back to the client connection
        transport
            .write_frame_for(&rand::random::<ConnectionId>())
            .await
            .unwrap();

        // Send garbage to fail the key exchange for new OTP
        transport.write_frame(Frame::new(b"hello")).await.unwrap();

        // Client should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn reconnect_should_fail_if_client_side_connection_unable_to_synchronize_with_server() {
        let (mut client, transport, _verifier, keychain) = setup_reconnect_scenario().await;
        let mut transport = FramedTransport::plain(transport);

        // Spawn a task to perform the client reconnection so we don't deadlock
        let task = tokio::spawn(async move { client.reconnect().await.unwrap() });

        // Send a version, although it'll be ignored by a reconnecting client
        send_server_version!(transport);

        // Perform first step of completing server-side of handshake
        transport.server_handshake().await.unwrap();

        // Receive reconnect data from client-side
        let (id, otp) = match transport.read_frame_as::<ConnectType>().await {
            Ok(Some(ConnectType::Reconnect { id, otp })) => (id, HeapSecretKey::from(otp)),
            x => panic!("Unexpected result: {x:?}"),
        };

        // Verify the id and OTP matches the one stored into our keychain from the setup
        assert!(
            keychain.has_key(id.to_string(), otp).await,
            "Wrong id or OTP"
        );

        // Send a new id back to the client connection
        transport
            .write_frame_for(&rand::random::<ConnectionId>())
            .await
            .unwrap();

        // Send garbage to fail the key exchange for new OTP
        transport.write_frame(Frame::new(b"hello")).await.unwrap();

        // Client should fail
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn reconnect_should_succeed_if_client_side_connection_fully_connects_and_synchronizes_with_server()
     {
        let (mut client, transport, _verifier, keychain) = setup_reconnect_scenario().await;
        let mut transport = FramedTransport::plain(transport);

        // Copy client backup for verification later
        let client_backup = client.transport().backup.clone();

        // Spawn a task to perform the client reconnection so we don't deadlock
        let task = tokio::spawn(async move {
            client.reconnect().await.unwrap();
            client
        });

        // Send a version, although it'll be ignored by a reconnecting client
        send_server_version!(transport);

        // Perform first step of completing server-side of handshake
        transport.server_handshake().await.unwrap();

        // Receive reconnect data from client-side
        let (id, otp) = match transport.read_frame_as::<ConnectType>().await {
            Ok(Some(ConnectType::Reconnect { id, otp })) => (id, HeapSecretKey::from(otp)),
            x => panic!("Unexpected result: {x:?}"),
        };

        // Retrieve server backup
        let backup = keychain
            .remove_if_has_key(id.to_string(), otp)
            .await
            .into_ok()
            .expect("Invalid id or OTP")
            .await
            .expect("Failed to retrieve backup");

        // Perform key exchange
        let otp = transport.exchange_keys().await.unwrap();

        // Perform synchronization after restoring backup
        transport.backup = backup;
        transport.synchronize().await.unwrap();

        // Client should succeed
        let mut client = task.await.unwrap();
        assert_eq!(client.otp(), Some(&otp.into_heap_secret_key()));

        // Verify client backup sent/received count was not modified (stored frames may be
        // truncated, though)
        assert_eq!(
            client.transport().backup.sent_cnt(),
            client_backup.sent_cnt(),
            "Client backup sent cnt altered"
        );
        assert_eq!(
            client.transport().backup.received_cnt(),
            client_backup.received_cnt(),
            "Client backup received cnt altered"
        );

        // Verify that client can send a frame and receive a frame, and that there is
        // nothing unexpected in the buffers on either side
        client.write_frame(Frame::new(b"hello")).await.unwrap();
        assert_eq!(transport.read_frame().await.unwrap().unwrap(), b"hello");
        transport.write_frame(Frame::new(b"goodbye")).await.unwrap();
        assert_eq!(client.read_frame().await.unwrap().unwrap(), b"goodbye");
    }

    #[test(tokio::test)]
    async fn reconnect_should_fail_if_connection_is_server_side() {
        let mut connection = Connection::Server {
            id: rand::random(),
            tx: oneshot::channel().0,
            transport: FramedTransport::pair(100).0,
        };

        assert_eq!(
            connection.reconnect().await.unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[test(tokio::test)]
    async fn client_server_returning_connection_e2e_should_reestablish_connection() {
        let (mut client, transport, verifier, keychain) = setup_reconnect_scenario().await;

        // Spawn a task to perform the server reconnection so we don't deadlock
        let task = tokio::spawn(async move {
            Connection::server(transport, &verifier, keychain, server_version!())
                .await
                .expect("Failed to connect from server")
        });

        // Reconnect and verify that the connection still works
        client
            .reconnect()
            .await
            .expect("Failed to reconnect from client");

        // Ensure the server is established and then drop it
        let mut server = task.await.unwrap();

        // Test out the connection
        client.write_frame(Frame::new(b"hello")).await.unwrap();
        assert_eq!(server.read_frame().await.unwrap().unwrap(), b"hello");
        server.write_frame(Frame::new(b"goodbye")).await.unwrap();
        assert_eq!(client.read_frame().await.unwrap().unwrap(), b"goodbye");
    }
}
