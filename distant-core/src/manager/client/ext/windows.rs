use crate::{DistantManagerClient, DistantManagerClientConfig};
use async_trait::async_trait;
use distant_net::common::{Codec, FramedTransport, WindowsPipeTransport};
use std::{
    convert,
    ffi::{OsStr, OsString},
};
use tokio::{io, time::Duration};

#[async_trait]
pub trait WindowsPipeDistantManagerClientExt {
    /// Connect to a server listening on a Windows pipe at the specified address
    /// using the given codec
    async fn connect<A, C>(
        config: DistantManagerClientConfig,
        addr: A,
        codec: C,
    ) -> io::Result<DistantManagerClient>
    where
        A: AsRef<OsStr> + Send,
        C: Codec + Send + 'static;

    /// Connect to a server listening on a Windows pipe at the specified address
    /// via `\\.\pipe\{name}` using the given codec
    async fn connect_local<N, C>(
        config: DistantManagerClientConfig,
        name: N,
        codec: C,
    ) -> io::Result<DistantManagerClient>
    where
        N: AsRef<OsStr> + Send,
        C: Codec + Send + 'static,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::connect(config, addr, codec).await
    }

    /// Connect to a server listening on a Windows pipe at the specified address
    /// using the given codec, timing out after duration has passed
    async fn connect_timeout<A, C>(
        config: DistantManagerClientConfig,
        addr: A,
        codec: C,
        duration: Duration,
    ) -> io::Result<DistantManagerClient>
    where
        A: AsRef<OsStr> + Send,
        C: Codec + Send + 'static,
    {
        tokio::time::timeout(duration, Self::connect(config, addr, codec))
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
            .and_then(convert::identity)
    }

    /// Connect to a server listening on a Windows pipe at the specified address
    /// via `\\.\pipe\{name}` using the given codec, timing out after duration has passed
    async fn connect_local_timeout<N, C>(
        config: DistantManagerClientConfig,
        name: N,
        codec: C,
        duration: Duration,
    ) -> io::Result<DistantManagerClient>
    where
        N: AsRef<OsStr> + Send,
        C: Codec + Send + 'static,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::connect_timeout(config, addr, codec, duration).await
    }
}

#[async_trait]
impl WindowsPipeDistantManagerClientExt for DistantManagerClient {
    async fn connect<A, C>(
        config: DistantManagerClientConfig,
        addr: A,
        codec: C,
    ) -> io::Result<DistantManagerClient>
    where
        A: AsRef<OsStr> + Send,
        C: Codec + Send + 'static,
    {
        let a = addr.as_ref();
        let transport = WindowsPipeTransport::connect(a).await?;
        let transport = FramedTransport::new(transport, codec);
        Ok(DistantManagerClient::new(config, transport)?)
    }
}
