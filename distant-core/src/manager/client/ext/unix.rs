use crate::{DistantManagerClient, DistantManagerClientConfig};
use async_trait::async_trait;
use distant_net::{Codec, FramedTransport, UnixSocketTransport};
use std::{convert, path::Path};
use tokio::{io, time::Duration};

#[async_trait]
pub trait UnixSocketDistantManagerClientExt {
    /// Connect to a proxy unix socket
    async fn connect<P, C>(
        config: DistantManagerClientConfig,
        path: P,
        codec: C,
    ) -> io::Result<DistantManagerClient>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + 'static;

    /// Connect to a proxy unix socket, timing out after duration has passed
    async fn connect_timeout<P, C>(
        config: DistantManagerClientConfig,
        path: P,
        codec: C,
        duration: Duration,
    ) -> io::Result<DistantManagerClient>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + 'static,
    {
        tokio::time::timeout(duration, Self::connect(config, path, codec))
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
            .and_then(convert::identity)
    }
}

#[async_trait]
impl UnixSocketDistantManagerClientExt for DistantManagerClient {
    /// Connect to a proxy unix socket
    async fn connect<P, C>(
        config: DistantManagerClientConfig,
        path: P,
        codec: C,
    ) -> io::Result<DistantManagerClient>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + 'static,
    {
        let p = path.as_ref();
        let transport = UnixSocketTransport::connect(p).await?;
        let transport = FramedTransport::new(transport, codec);
        Ok(DistantManagerClient::new(config, transport)?)
    }
}
