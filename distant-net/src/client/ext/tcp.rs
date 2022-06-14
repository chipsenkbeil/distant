use crate::{Client, Codec, FramedTransport, TcpTransport};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::{convert, net::SocketAddr};
use tokio::{io, time::Duration};

#[async_trait]
pub trait TcpClientExt<T, U>
where
    T: Serialize + Send + Sync,
    U: DeserializeOwned + Send + Sync,
{
    /// Connect to a remote TCP server using the provided information
    async fn connect<C>(addr: SocketAddr, codec: C) -> io::Result<Client<T, U>>
    where
        C: Codec + Send + 'static;

    /// Connect to a remote TCP server, timing out after duration has passed
    async fn connect_timeout<C>(
        addr: SocketAddr,
        codec: C,
        duration: Duration,
    ) -> io::Result<Client<T, U>>
    where
        C: Codec + Send + 'static,
    {
        tokio::time::timeout(duration, Self::connect(addr, codec))
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
            .and_then(convert::identity)
    }
}

#[async_trait]
impl<T, U> TcpClientExt<T, U> for Client<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Connect to a remote TCP server using the provided information
    async fn connect<C>(addr: SocketAddr, codec: C) -> io::Result<Client<T, U>>
    where
        C: Codec + Send + 'static,
    {
        let transport = TcpTransport::connect(addr).await?;
        let transport = FramedTransport::new(transport, codec);
        Self::from_framed_transport(transport)
    }
}
