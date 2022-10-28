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

use crate::client::{Client, ReconnectStrategy};
use crate::common::{authentication::AuthHandler, Connection, Transport};
use serde::{de::DeserializeOwned, Serialize};
use std::{convert, future::Future, io, time::Duration};

/// Builder for a [`Client`]
pub struct ClientBuilder<H, T> {
    auth_handler: H,
    reconnect_strategy: ReconnectStrategy,
    transport: T,
    timeout: Option<Duration>,
}

impl<H, T> ClientBuilder<H, T> {
    pub fn auth_handler<U>(self, auth_handler: U) -> ClientBuilder<U, T> {
        ClientBuilder {
            auth_handler,
            reconnect_strategy: self.reconnect_strategy,
            transport: self.transport,
            timeout: self.timeout,
        }
    }

    pub fn reconnect_strategy(self, reconnect_strategy: ReconnectStrategy) -> ClientBuilder<H, T> {
        ClientBuilder {
            auth_handler: self.auth_handler,
            reconnect_strategy,
            transport: self.transport,
            timeout: self.timeout,
        }
    }

    pub async fn try_transport<U>(
        self,
        f: impl Future<Output = io::Result<U>>,
    ) -> io::Result<ClientBuilder<H, U>> {
        let timeout = self.timeout.as_ref().copied();
        Ok(self.transport(match timeout {
            Some(duration) => tokio::time::timeout(duration, f)
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
                .and_then(convert::identity)?,
            None => f.await?,
        }))
    }

    pub fn transport<U>(self, transport: U) -> ClientBuilder<H, U> {
        ClientBuilder {
            auth_handler: self.auth_handler,
            reconnect_strategy: self.reconnect_strategy,
            transport,
            timeout: self.timeout,
        }
    }

    pub fn timeout(self, timeout: impl Into<Option<Duration>>) -> Self {
        Self {
            auth_handler: self.auth_handler,
            reconnect_strategy: self.reconnect_strategy,
            transport: self.transport,
            timeout: timeout.into(),
        }
    }
}

impl ClientBuilder<(), ()> {
    pub fn new() -> Self {
        Self {
            auth_handler: (),
            reconnect_strategy: ReconnectStrategy::default(),
            transport: (),
            timeout: None,
        }
    }
}

impl Default for ClientBuilder<(), ()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H, T> ClientBuilder<H, T>
where
    H: AuthHandler + Send,
    T: Transport + Send + Sync + 'static,
{
    pub async fn connect<U, V>(self) -> io::Result<Client<U, V>>
    where
        U: Send + Sync + Serialize + 'static,
        V: Send + Sync + DeserializeOwned + 'static,
    {
        let auth_handler = self.auth_handler;
        let retry_strategy = self.reconnect_strategy;
        let timeout = self.timeout;
        let transport = self.transport;

        let f = async move {
            let connection = Connection::client(transport, auth_handler).await?;
            Ok(Client::spawn(connection, retry_strategy))
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
