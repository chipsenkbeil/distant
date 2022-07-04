use crate::DistantManager;
use distant_net::{
    Codec, FramedTransport, IntoSplit, MappedListener, PortRange, ServerExt, TcpListener,
    TcpServerRef,
};
use std::{io, net::IpAddr};

impl DistantManager {
    pub async fn start_tcp<P, C>(self, addr: IpAddr, port: P, codec: C) -> io::Result<TcpServerRef>
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
        let inner = self.start(listener)?;
        Ok(TcpServerRef::new(addr, port, inner))
    }
}
