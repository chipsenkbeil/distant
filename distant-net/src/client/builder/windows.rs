use crate::{
    auth::{AuthHandler, Authenticate},
    Client, FramedTransport, WindowsPipeTransport,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    convert,
    ffi::{OsStr, OsString},
};
use tokio::{io, time::Duration};

/// Builder for a client that will connect over a Windows pipe
pub struct WindowsPipeClientBuilder<T> {
    auth_handler: T,
    local: bool,
    timeout: Option<Duration>,
}

impl<T> WindowsPipeClientBuilder<T> {
    pub fn auth_handler<A: AuthHandler>(self, auth_handler: A) -> WindowsPipeClientBuilder<A> {
        WindowsPipeClientBuilder {
            auth_handler,
            local: self.local,
            timeout: self.timeout,
        }
    }

    /// If true, will connect to a server listening on a Windows pipe at the specified address
    /// via `\\.\pipe\{name}`; otherwise, will connect using the address verbatim.
    pub fn local(self, local: bool) -> Self {
        Self {
            auth_handler: self.auth_handler,
            local,
            timeout: self.timeout,
        }
    }

    pub fn timeout(self, timeout: impl Into<Option<Duration>>) -> Self {
        Self {
            auth_handler: self.auth_handler,
            local: self.local,
            timeout: timeout.into(),
        }
    }
}

impl WindowsPipeClientBuilder<()> {
    pub fn new() -> Self {
        Self {
            auth_handler: (),
            local: false,
            timeout: None,
        }
    }
}

impl Default for WindowsPipeClientBuilder<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: AuthHandler + Send> WindowsPipeClientBuilder<A> {
    pub async fn connect<T, U>(self, addr: impl AsRef<OsStr> + Send) -> io::Result<Client<T, U>>
    where
        T: Send + Sync + Serialize + 'static,
        U: Send + Sync + DeserializeOwned + 'static,
    {
        let auth_handler = self.auth_handler;
        let timeout = self.timeout;

        let f = async move {
            let transport = if self.local {
                let mut full_addr = OsString::from(r"\\.\pipe\");
                full_addr.push(addr.as_ref());
                WindowsPipeTransport::connect(full_addr).await?
            } else {
                WindowsPipeTransport::connect(addr.as_ref()).await?
            };

            // Establish our framed transport, perform a handshake to set the codec, and do
            // authentication to ensure the connection can be used
            let mut transport = FramedTransport::<_>::plain(transport);
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
