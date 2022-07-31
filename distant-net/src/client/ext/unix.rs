use crate::{Client, Codec, FramedTransport, IntoSplit, UnixSocketTransport};
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
    async fn connect<P, C>(path: P, codec: C) -> io::Result<Client<T, U>>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + 'static;

    /// Connect to a proxy unix socket, timing out after duration has passed
    async fn connect_timeout<P, C>(
        path: P,
        codec: C,
        duration: Duration,
    ) -> io::Result<Client<T, U>>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + 'static,
    {
        tokio::time::timeout(duration, Self::connect(path, codec))
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
    async fn connect<P, C>(path: P, codec: C) -> io::Result<Client<T, U>>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + 'static,
    {
        let p = path.as_ref();
        let transport = UnixSocketTransport::connect(p).await?;
        let transport = FramedTransport::new(transport, codec);
        let (writer, reader) = transport.into_split();
        Ok(Client::new(writer, reader)?)
    }
}
