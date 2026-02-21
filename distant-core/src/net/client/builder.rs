mod tcp;
pub use tcp::*;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;

use std::time::Duration;
use std::{convert, io};

use crate::auth::AuthHandler;
use async_trait::async_trait;
#[cfg(windows)]
pub use windows::*;

use super::ClientConfig;
use crate::net::client::{Client, UntypedClient};
use crate::net::common::{Connection, Transport, Version};

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
    config: ClientConfig,
    connect_timeout: Option<Duration>,
    version: Version,
}

impl<H, C> ClientBuilder<H, C> {
    /// Configure the authentication handler to use when connecting to a server.
    pub fn auth_handler<U>(self, auth_handler: U) -> ClientBuilder<U, C> {
        ClientBuilder {
            auth_handler,
            config: self.config,
            connector: self.connector,
            connect_timeout: self.connect_timeout,
            version: self.version,
        }
    }

    /// Configure the client-local configuration details.
    pub fn config(self, config: ClientConfig) -> Self {
        Self {
            auth_handler: self.auth_handler,
            config,
            connector: self.connector,
            connect_timeout: self.connect_timeout,
            version: self.version,
        }
    }

    /// Configure the connector to use to facilitate connecting to a server.
    pub fn connector<U>(self, connector: U) -> ClientBuilder<H, U> {
        ClientBuilder {
            auth_handler: self.auth_handler,
            config: self.config,
            connector,
            connect_timeout: self.connect_timeout,
            version: self.version,
        }
    }

    /// Configure a maximum duration to wait for a connection to a server to complete.
    pub fn connect_timeout(self, connect_timeout: impl Into<Option<Duration>>) -> Self {
        Self {
            auth_handler: self.auth_handler,
            config: self.config,
            connector: self.connector,
            connect_timeout: connect_timeout.into(),
            version: self.version,
        }
    }

    /// Configure the version of the client.
    pub fn version(self, version: Version) -> Self {
        Self {
            auth_handler: self.auth_handler,
            config: self.config,
            connector: self.connector,
            connect_timeout: self.connect_timeout,
            version,
        }
    }
}

impl ClientBuilder<(), ()> {
    pub fn new() -> Self {
        Self {
            auth_handler: (),
            config: Default::default(),
            connector: (),
            connect_timeout: None,
            version: Default::default(),
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
        let config = self.config;
        let connect_timeout = self.connect_timeout;
        let version = self.version;

        let f = async move {
            let transport = match connect_timeout {
                Some(duration) => tokio::time::timeout(duration, self.connector.connect())
                    .await
                    .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
                    .and_then(convert::identity)?,
                None => self.connector.connect().await?,
            };
            let connection = Connection::client(transport, auth_handler, version).await?;
            Ok(UntypedClient::spawn(connection, config))
        };

        match connect_timeout {
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
