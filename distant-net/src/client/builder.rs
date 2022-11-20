mod tcp;
pub use tcp::*;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::*;

use crate::client::{Client, ReconnectStrategy, UntypedClient};
use crate::common::{authentication::AuthHandler, Connection, Transport};
use async_trait::async_trait;
use std::{convert, io, time::Duration};

/// Interface that performs the connection to produce a [`Transport`] for use by the [`Client`].
#[async_trait]
pub trait Connector {
    /// Type of transport produced by the connection.
    type Transport: Transport + 'static;

    async fn connect(self) -> io::Result<Self::Transport>;
}

#[async_trait]
impl<T: Transport + 'static> Connector for T {
    type Transport = T;

    async fn connect(self) -> io::Result<Self::Transport> {
        Ok(self)
    }
}

/// Builder for a [`Client`] or [`UntypedClient`].
pub struct ClientBuilder<H, C> {
    auth_handler: H,
    connector: C,
    reconnect_strategy: ReconnectStrategy,
    timeout: Option<Duration>,
}

impl<H, C> ClientBuilder<H, C> {
    pub fn auth_handler<U>(self, auth_handler: U) -> ClientBuilder<U, C> {
        ClientBuilder {
            auth_handler,
            connector: self.connector,
            reconnect_strategy: self.reconnect_strategy,
            timeout: self.timeout,
        }
    }

    pub fn connector<U>(self, connector: U) -> ClientBuilder<H, U> {
        ClientBuilder {
            auth_handler: self.auth_handler,
            connector,
            reconnect_strategy: self.reconnect_strategy,
            timeout: self.timeout,
        }
    }

    pub fn reconnect_strategy(self, reconnect_strategy: ReconnectStrategy) -> ClientBuilder<H, C> {
        ClientBuilder {
            auth_handler: self.auth_handler,
            connector: self.connector,
            reconnect_strategy,
            timeout: self.timeout,
        }
    }

    pub fn timeout(self, timeout: impl Into<Option<Duration>>) -> Self {
        Self {
            auth_handler: self.auth_handler,
            connector: self.connector,
            reconnect_strategy: self.reconnect_strategy,
            timeout: timeout.into(),
        }
    }
}

impl ClientBuilder<(), ()> {
    pub fn new() -> Self {
        Self {
            auth_handler: (),
            reconnect_strategy: ReconnectStrategy::default(),
            connector: (),
            timeout: None,
        }
    }
}

impl Default for ClientBuilder<(), ()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H, C> ClientBuilder<H, C>
where
    H: AuthHandler + Send,
    C: Connector,
{
    /// Establishes a connection with a remote server using the configured [`Transport`]
    /// and other settings, returning a new [`UntypedClient`] instance once the connection
    /// is fully established and authenticated.
    pub async fn connect_untyped(self) -> io::Result<UntypedClient> {
        let auth_handler = self.auth_handler;
        let retry_strategy = self.reconnect_strategy;
        let timeout = self.timeout;

        let f = async move {
            let transport = match timeout {
                Some(duration) => tokio::time::timeout(duration, self.connector.connect())
                    .await
                    .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
                    .and_then(convert::identity)?,
                None => self.connector.connect().await?,
            };
            let connection = Connection::client(transport, auth_handler).await?;
            Ok(UntypedClient::spawn(connection, retry_strategy))
        };

        match timeout {
            Some(duration) => tokio::time::timeout(duration, f)
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
                .and_then(convert::identity),
            None => f.await,
        }
    }

    /// Establishes a connection with a remote server using the configured [`Transport`] and other
    /// settings, returning a new [`Client`] instance once the connection is fully established and
    /// authenticated.
    pub async fn connect<T, U>(self) -> io::Result<Client<T, U>> {
        Ok(self.connect_untyped().await?.into_typed_client())
    }
}
