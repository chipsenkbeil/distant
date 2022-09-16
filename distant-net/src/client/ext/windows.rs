use crate::{Client, Codec, FramedTransport, WindowsPipeTransport};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    convert,
    ffi::{OsStr, OsString},
};
use tokio::{io, time::Duration};

#[async_trait]
pub trait WindowsPipeClientExt<T, U>
where
    T: Serialize + Send + Sync,
    U: DeserializeOwned + Send + Sync,
{
    /// Connect to a server listening on a Windows pipe at the specified address
    /// using the given codec
    async fn connect<A, C>(addr: A, codec: C) -> io::Result<Client<T, U>>
    where
        A: AsRef<OsStr> + Send,
        C: Codec + Send + 'static;

    /// Connect to a server listening on a Windows pipe at the specified address
    /// via `\\.\pipe\{name}` using the given codec
    async fn connect_local<N, C>(name: N, codec: C) -> io::Result<Client<T, U>>
    where
        N: AsRef<OsStr> + Send,
        C: Codec + Send + 'static,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::connect(addr, codec).await
    }

    /// Connect to a server listening on a Windows pipe at the specified address
    /// using the given codec, timing out after duration has passed
    async fn connect_timeout<A, C>(
        addr: A,
        codec: C,
        duration: Duration,
    ) -> io::Result<Client<T, U>>
    where
        A: AsRef<OsStr> + Send,
        C: Codec + Send + 'static,
    {
        tokio::time::timeout(duration, Self::connect(addr, codec))
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
            .and_then(convert::identity)
    }

    /// Connect to a server listening on a Windows pipe at the specified address
    /// via `\\.\pipe\{name}` using the given codec, timing out after duration has passed
    async fn connect_local_timeout<N, C>(
        name: N,
        codec: C,
        duration: Duration,
    ) -> io::Result<Client<T, U>>
    where
        N: AsRef<OsStr> + Send,
        C: Codec + Send + 'static,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::connect_timeout(addr, codec, duration).await
    }
}

#[async_trait]
impl<T, U> WindowsPipeClientExt<T, U> for Client<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    async fn connect<A, C>(addr: A, codec: C) -> io::Result<Client<T, U>>
    where
        A: AsRef<OsStr> + Send,
        C: Codec + Send + 'static,
    {
        let a = addr.as_ref();
        let transport = WindowsPipeTransport::connect(a).await?;
        let transport = FramedTransport::new(transport, codec);
        let (writer, reader) = transport.into_split();
        Ok(Client::new(writer, reader)?)
    }
}
