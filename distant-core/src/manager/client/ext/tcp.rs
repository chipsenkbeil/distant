use crate::{DistantManagerClient, DistantManagerClientConfig};
use async_trait::async_trait;
use distant_net::common::{Codec, FramedTransport, TcpTransport};
use std::{convert, net::SocketAddr};
use tokio::{io, time::Duration};

#[async_trait]
pub trait TcpDistantManagerClientExt {
    /// Connect to a remote TCP server using the provided information
    async fn connect<C>(
        config: DistantManagerClientConfig,
        addr: SocketAddr,
        codec: C,
    ) -> io::Result<DistantManagerClient>
    where
        C: Codec + Send + 'static;

    /// Connect to a remote TCP server, timing out after duration has passed
    async fn connect_timeout<C>(
        config: DistantManagerClientConfig,
        addr: SocketAddr,
        codec: C,
        duration: Duration,
    ) -> io::Result<DistantManagerClient>
    where
        C: Codec + Send + 'static,
    {
        tokio::time::timeout(duration, Self::connect(config, addr, codec))
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
            .and_then(convert::identity)
    }
}

#[async_trait]
impl TcpDistantManagerClientExt for DistantManagerClient {
    /// Connect to a remote TCP server using the provided information
    async fn connect<C>(
        config: DistantManagerClientConfig,
        addr: SocketAddr,
        codec: C,
    ) -> io::Result<DistantManagerClient>
    where
        C: Codec + Send + 'static,
    {
        let transport = TcpTransport::connect(addr).await?;
        let transport = FramedTransport::new(transport, codec);
        Self::new(config, transport)
    }
}
