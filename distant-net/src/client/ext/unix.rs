use crate::{Client, FramedTransport, UnixSocketTransport};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::{convert, path::Path};
use tokio::{io, time::Duration};

#[async_trait]
pub trait UnixSocketClientExt<T, U>
where
    T: Serialize + Send + Sync,
    U: DeserializeOwned + Send + Sync,
{
    /// Connect to a proxy unix socket
    async fn connect<P>(path: P) -> io::Result<Client<T, U>>
    where
        P: AsRef<Path> + Send;

    /// Connect to a proxy unix socket, timing out after duration has passed
    async fn connect_timeout<P>(path: P, duration: Duration) -> io::Result<Client<T, U>>
    where
        P: AsRef<Path> + Send,
    {
        tokio::time::timeout(duration, Self::connect(path))
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
            .and_then(convert::identity)
    }
}

#[async_trait]
impl<T, U> UnixSocketClientExt<T, U> for Client<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Connect to a proxy unix socket
    async fn connect<P>(path: P) -> io::Result<Client<T, U>>
    where
        P: AsRef<Path> + Send,
    {
        let p = path.as_ref();
        let transport = UnixSocketTransport::connect(p).await?;

        // Establish our framed transport and perform a handshake to set the codec
        // NOTE: Using default capacity
        let mut transport = FramedTransport::<_>::plain(transport);
        transport.client_handshake().await?;

        Ok(Client::new(transport))
    }
}
