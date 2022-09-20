use crate::{Client, FramedTransport, WindowsPipeTransport};
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
    async fn connect<A>(addr: A) -> io::Result<Client<T, U>>
    where
        A: AsRef<OsStr> + Send;

    /// Connect to a server listening on a Windows pipe at the specified address
    /// via `\\.\pipe\{name}` using the given codec
    async fn connect_local<N>(name: N) -> io::Result<Client<T, U>>
    where
        N: AsRef<OsStr> + Send,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::connect(addr).await
    }

    /// Connect to a server listening on a Windows pipe at the specified address
    /// using the given codec, timing out after duration has passed
    async fn connect_timeout<A>(addr: A, duration: Duration) -> io::Result<Client<T, U>>
    where
        A: AsRef<OsStr> + Send,
    {
        tokio::time::timeout(duration, Self::connect(addr))
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
            .and_then(convert::identity)
    }

    /// Connect to a server listening on a Windows pipe at the specified address
    /// via `\\.\pipe\{name}`, timing out after duration has passed
    async fn connect_local_timeout<N>(name: N, duration: Duration) -> io::Result<Client<T, U>>
    where
        N: AsRef<OsStr> + Send,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::connect_timeout(addr, duration).await
    }
}

#[async_trait]
impl<T, U> WindowsPipeClientExt<T, U> for Client<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    async fn connect<A>(addr: A) -> io::Result<Client<T, U>>
    where
        A: AsRef<OsStr> + Send,
    {
        let a = addr.as_ref();
        let transport = WindowsPipeTransport::connect(a).await?;

        // Establish our framed transport and perform a handshake to set the codec
        // NOTE: Using default capacity
        let mut transport = FramedTransport::<_>::plain(transport);
        transport.client_handshake().await?;

        Ok(Client::new(transport))
    }
}
