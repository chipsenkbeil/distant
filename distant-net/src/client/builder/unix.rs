use crate::{auth::AuthHandler, Client, ClientBuilder, UnixSocketTransport};
use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;
use tokio::{io, time::Duration};

/// Builder for a client that will connect over a Unix socket
pub struct UnixSocketClientBuilder<T>(ClientBuilder<T, ()>);

impl<T> UnixSocketClientBuilder<T> {
    pub fn auth_handler<A: AuthHandler>(self, auth_handler: A) -> UnixSocketClientBuilder<A> {
        UnixSocketClientBuilder(self.0.auth_handler(auth_handler))
    }

    pub fn timeout(self, timeout: impl Into<Option<Duration>>) -> Self {
        Self(self.0.timeout(timeout))
    }
}

impl UnixSocketClientBuilder<()> {
    pub fn new() -> Self {
        Self(ClientBuilder::new())
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
        self.0
            .try_transport(UnixSocketTransport::connect(path.as_ref()))
            .await?
            .connect()
            .await
    }
}
