use crate::{DistantManager, DistantManagerConfig};
use distant_net::{
    Codec, FramedTransport, IntoSplit, MappedListener, PortRange, TcpListener, TcpServerRef,
};
use std::{io, net::IpAddr};

impl DistantManager {
    /// Start a new server by binding to the given IP address and one of the ports in the
    /// specified range, mapping all connections to use the given codec
    pub async fn start_tcp<P, C>(
        config: DistantManagerConfig,
        addr: IpAddr,
        port: P,
        codec: C,
    ) -> io::Result<TcpServerRef>
    where
        P: Into<PortRange> + Send,
        C: Codec + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(addr, port).await?;
        let port = listener.port();

        let listener = MappedListener::new(listener, move |transport| {
            let transport = FramedTransport::new(transport, codec.clone());
            transport.into_split()
        });
        let inner = DistantManager::start(config, listener)?;
        Ok(TcpServerRef::new(addr, port, Box::new(inner)))
    }
}
