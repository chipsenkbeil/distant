use crate::client::{Client, ClientBuilder};
use crate::common::{authentication::AuthHandler, TcpTransport};
use serde::{de::DeserializeOwned, Serialize};
use tokio::{io, net::ToSocketAddrs, time::Duration};

/// Builder for a client that will connect over TCP
pub struct TcpClientBuilder<T>(ClientBuilder<T, ()>);

impl<T> TcpClientBuilder<T> {
    pub fn auth_handler<A: AuthHandler>(self, auth_handler: A) -> TcpClientBuilder<A> {
        TcpClientBuilder(self.0.auth_handler(auth_handler))
    }

    pub fn timeout(self, timeout: impl Into<Option<Duration>>) -> Self {
        Self(self.0.timeout(timeout))
    }
}

impl TcpClientBuilder<()> {
    pub fn new() -> Self {
        Self(ClientBuilder::new())
    }
}

impl Default for TcpClientBuilder<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<A: AuthHandler + Send> TcpClientBuilder<A> {
    pub async fn connect<T, U>(self, addr: impl ToSocketAddrs) -> io::Result<Client<T, U>>
    where
        T: Send + Sync + Serialize + 'static,
        U: Send + Sync + DeserializeOwned + 'static,
    {
        self.0
            .try_transport(TcpTransport::connect(addr))
            .await?
            .connect()
            .await
    }
}
