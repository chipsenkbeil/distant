use crate::client::{Client, ClientBuilder};
use crate::common::{authentication::AuthHandler, WindowsPipeTransport};
use serde::{de::DeserializeOwned, Serialize};
use std::ffi::{OsStr, OsString};
use tokio::{io, time::Duration};

/// Builder for a client that will connect over a Windows pipe
pub struct WindowsPipeClientBuilder<T> {
    inner: ClientBuilder<T, ()>,
    local: bool,
}

impl<T> WindowsPipeClientBuilder<T> {
    pub fn auth_handler<A: AuthHandler>(self, auth_handler: A) -> WindowsPipeClientBuilder<A> {
        WindowsPipeClientBuilder {
            inner: self.inner.auth_handler(auth_handler),
            local: self.local,
        }
    }

    /// If true, will connect to a server listening on a Windows pipe at the specified address
    /// via `\\.\pipe\{name}`; otherwise, will connect using the address verbatim.
    pub fn local(self, local: bool) -> Self {
        Self {
            inner: self.inner,
            local,
        }
    }

    pub fn timeout(self, timeout: impl Into<Option<Duration>>) -> Self {
        Self {
            inner: self.inner.timeout(timeout),
            local: self.local,
        }
    }
}

impl WindowsPipeClientBuilder<()> {
    pub fn new() -> Self {
        Self {
            inner: ClientBuilder::new(),
            local: false,
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
        let local = self.local;
        self.0
            .try_transport(if local {
                let mut full_addr = OsString::from(r"\\.\pipe\");
                full_addr.push(addr.as_ref());
                WindowsPipeTransport::connect(full_addr)
            } else {
                WindowsPipeTransport::connect(addr.as_ref())
            })
            .await?
            .connect()
            .await
    }
}
