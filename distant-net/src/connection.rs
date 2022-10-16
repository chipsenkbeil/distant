use super::{
    auth::{AuthHandler, Authenticate, Keychain, Verifier},
    FramedTransport, HeapSecretKey, Reconnectable, Transport,
};
use async_trait::async_trait;
use log::*;
use serde::{Deserialize, Serialize};
use std::io;
use std::ops::{Deref, DerefMut};

/// Id of the connection
pub type ConnectionId = u32;

/// Represents a connection from either the client or server side
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

#[async_trait]
impl<T> Reconnectable for Connection<T>
where
    T: Transport + Send + Sync,
{
    /// Attempts to re-establish a connection.
    ///
    /// ### Client
    ///
    /// For a client, this means performing an actual [`reconnect`] on the underlying
    /// [`Transport`], re-establishing an encrypted codec, submitting a request to the server to
    /// reauthenticate using a previously-derived OTP, and refreshing the connection id and OTP for
    /// use in a future reauthentication.
    ///
    /// ### Server
    ///
    /// For a server, this will fail as unsupported.
    ///
    /// [`reconnect`]: Reconnectable::reconnect
    async fn reconnect(&mut self) -> io::Result<()> {
        async fn reconnect_client<T: Transport + Send + Sync>(
            id: &mut ConnectionId,
            reauth_otp: &mut HeapSecretKey,
            transport: &mut FramedTransport<T>,
        ) -> io::Result<()> {
            // Re-establish a raw connection
            debug!("[Conn {id}] Re-establishing connection");
            Reconnectable::reconnect(transport).await?;

            // Perform a handshake to ensure that the connection is properly established and encrypted
            debug!("[Conn {id}] Performing handshake");
            transport.client_handshake().await?;

            // Communicate that we are an existing connection
            debug!("[Conn {id}] Performing re-authentication");
            transport
                .write_frame_for(&ConnectType::Reconnect {
                    id: *id,
                    otp: reauth_otp.unprotected_as_bytes().to_vec(),
                })
                .await?;

            // Receive the new id for the connection
            // NOTE: If we fail re-authentication above,
            //       this will fail as the connection is dropped
            debug!("[Conn {id}] Receiving new connection id");
            let new_id = transport
                .read_frame_as::<ConnectionId>()
                .await?
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::Other, "Missing connection id frame")
                })?;
            debug!("[Conn {id}] Resetting id to {new_id}");

            // Derive an OTP for reauthentication
            debug!("[Conn {new_id}] Deriving future OTP for reauthentication");
            let new_reauth_otp = transport.exchange_keys().await?.into_heap_secret_key();

            // Update our connection's id and reauth OTP
            *id = new_id;
            *reauth_otp = new_reauth_otp;
            Ok(())
        }

        match self {
            Self::Client {
                id,
                transport,
                reauth_otp,
            } => {
                // Freeze our backup as we don't want the connection logic to alter it
                transport.backup.freeze();

                // Attempt to perform the reconnection and unfreeze our backup regardless of the
                // result
                let result = reconnect_client(id, reauth_otp, transport).await;
                transport.backup.unfreeze();
                result?;

                // Perform synchronization
                transport.synchronize().await?;

                Ok(())
            }

            Self::Server { .. } => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Server connection cannot reconnect",
            )),
        }
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

impl<T> Connection<T> {
    /// Consume the connection and return the underlying transport.
    pub fn into_transport(self) -> FramedTransport<T> {
        match self {
            Self::Client { transport, .. } => transport,
            Self::Server { transport, .. } => transport,
        }
    }
}

impl<T> Connection<T>
where
    T: Transport + Send + Sync,
{
    /// Transforms a raw [`Transport`] into an established [`Connection`] from the client-side by
    /// performing the following:
    ///
    /// 1. Handshakes to derive the appropriate [`Codec`](crate::Codec) to use
    /// 2. Authenticates the established connection to ensure it is valid
    /// 3. Restores pre-existing state using the provided backup, replaying any missing frames and
    ///    receiving any frames from the other side
    pub async fn client<H: AuthHandler + Send>(transport: T, handler: H) -> io::Result<Self> {
        let id: ConnectionId = rand::random();

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
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::Other, "Missing connection id frame")
                })?;
            debug!("[Conn {id}] Resetting id to {new_id}");
            new_id
        };

        // Authenticate the transport with the server-side
        debug!("[Conn {id}] Performing authentication");
        transport.authenticate(handler).await?;

        // Derive an OTP for reauthentication
        debug!("[Conn {id}] Deriving future OTP for reauthentication");
        let reauth_otp = transport.exchange_keys().await?.into_heap_secret_key();

        Ok(Self::Client {
            id,
            reauth_otp,
            transport,
        })
    }

    /// Transforms a raw [`Transport`] into an established [`Connection`] from the server-side by
    /// performing the following:
    ///
    /// 1. Handshakes to derive the appropriate [`Codec`](crate::Codec) to use
    /// 2. Authenticates the established connection to ensure it is valid
    /// 3. Restores pre-existing state using the provided backup, replaying any missing frames and
    ///    receiving any frames from the other side
    pub async fn server(transport: T, verifier: &Verifier, keychain: Keychain) -> io::Result<Self> {
        let id: ConnectionId = rand::random();

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
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Missing connection type frame"))?;

        // Based on the connection type, we either try to find and validate an existing connection
        // or we perform normal verification
        match connection_type {
            ConnectType::Connect => {
                // Communicate the connection id
                debug!("[Conn {id}] Telling other side to change connection id");
                transport.write_frame_for(&id).await?;

                // Perform authentication to ensure the connection is valid
                debug!("[Conn {id}] Verifying connection");
                verifier.verify(&mut transport).await?;
            }
            ConnectType::Reconnect { id: other_id, otp } => {
                let reauth_otp = HeapSecretKey::from(otp);

                debug!("[Conn {id}] Checking if {other_id} exists");
                if let Some(otp) = keychain.remove(other_id.to_string()).await {
                    debug!("[Conn {id}] Checking if OTP matches for {other_id}");
                    if reauth_otp != otp {
                        // Re-add the existing OTP since we didn't match
                        keychain.insert(other_id.to_string(), otp).await;

                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "Invalid OTP",
                        ));
                    }
                }

                // Communicate the connection id
                debug!("[Conn {id}] Telling other side to change connection id");
                transport.write_frame_for(&id).await?;
            }
        }

        // Derive an OTP for reauthentication
        debug!("[Conn {id}] Deriving future OTP for reauthentication");
        let reauth_otp = transport.exchange_keys().await?.into_heap_secret_key();

        // Store the id and OTP in our database
        keychain.insert(id.to_string(), reauth_otp).await;

        Ok(Self::Server { id, transport })
    }
}
