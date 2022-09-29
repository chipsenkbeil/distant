use crate::{
    auth::{AuthHandler, Authenticate},
    Client, FramedTransport, UnixSocketTransport,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{convert, path::Path};
use tokio::{io, time::Duration};

/// Builder for a client that will connect over a Unix socket
pub struct UnixSocketClientBuilder<T> {
    auth_handler: T,
    timeout: Option<Duration>,
}

impl<T> UnixSocketClientBuilder<T> {
    pub fn auth_handler<A: AuthHandler>(self, auth_handler: A) -> UnixSocketClientBuilder<A> {
        UnixSocketClientBuilder {
            auth_handler,
            timeout: self.timeout,
        }
    }

    pub fn timeout(self, timeout: impl Into<Option<Duration>>) -> Self {
        Self {
            auth_handler: self.auth_handler,
            timeout: timeout.into(),
        }
    }
}

impl UnixSocketClientBuilder<()> {
    pub fn new() -> Self {
        Self {
            auth_handler: (),
            timeout: None,
        }
    }
}

impl Default for UnixSocketClientBuilder<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: AuthHandler + Send> UnixSocketClientBuilder<A> {
    pub async fn connect<T, U>(self, path: impl AsRef<Path> + Send) -> io::Result<Client<T, U>>
    where
        T: Send + Sync + Serialize + 'static,
        U: Send + Sync + DeserializeOwned + 'static,
    {
        let auth_handler = self.auth_handler;
        let timeout = self.timeout;

        let f = async move {
            let p = path.as_ref();
            let transport = UnixSocketTransport::connect(p).await?;

            // Establish our framed transport, perform a handshake to set the codec, and do
            // authentication to ensure the connection can be used
            let mut transport = FramedTransport::plain(transport);
            transport.client_handshake().await?;
            transport.authenticate(auth_handler).await?;

            Ok(Client::new(transport))
        };

        match timeout {
            Some(duration) => tokio::time::timeout(duration, f)
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
                .and_then(convert::identity),
            None => f.await,
        }
    }
}
