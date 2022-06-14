use crate::{
    Codec, FramedTransport, IntoSplit, MappedListener, PortRange, Server, ServerExt, TcpListener,
    TcpServerRef,
};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::{io, net::IpAddr};

/// Extension trait to provide a reference implementation of starting a TCP server
/// that will listen for new connections and process them using the [`Server`] implementation
#[async_trait]
pub trait TcpServerExt {
    type Request;
    type Response;

    /// Start a new server using the provided listener
    async fn start<P, C>(addr: IpAddr, port: P, codec: C) -> io::Result<TcpServerRef>
    where
        P: Into<PortRange> + Send,
        C: Codec + Send + Sync + 'static;
}

#[async_trait]
impl<S, Req, Res, Gdata, Ldata> TcpServerExt for S
where
    S: Server<Request = Req, Response = Res, GlobalData = Gdata, LocalData = Ldata>,
    Req: DeserializeOwned + Send + Sync,
    Res: Serialize + Send + 'static,
    Gdata: Default + Send + Sync + 'static,
    Ldata: Default + Send + Sync + 'static,
{
    type Request = Req;
    type Response = Res;

    async fn start<P, C>(addr: IpAddr, port: P, codec: C) -> io::Result<TcpServerRef>
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
        let inner = <S as ServerExt>::start(listener)?;
        Ok(TcpServerRef { addr, port, inner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Client, PlainCodec, Request, ServerCtx, TcpClientExt};
    use std::net::{Ipv6Addr, SocketAddr};

    pub struct TestServer;

    #[async_trait]
    impl Server for TestServer {
        type Request = String;
        type Response = String;
        type GlobalData = ();
        type LocalData = ();

        async fn on_request(
            ctx: &ServerCtx<Self::Request, Self::Response, Self::GlobalData, Self::LocalData>,
        ) -> io::Result<()> {
            // Echo back what we received
            ctx.reply
                .send(ctx.request.payload.to_string())
                .await
                .unwrap();

            Ok(())
        }
    }

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        let server =
            <TestServer as TcpServerExt>::start(IpAddr::V6(Ipv6Addr::LOCALHOST), 0, PlainCodec)
                .await
                .expect("Failed to start TCP server");

        let mut client: Client<String, String> = Client::connect(
            SocketAddr::from((server.ip_addr(), server.port())),
            PlainCodec,
        )
        .await
        .expect("Client failed to connect");

        let response = client
            .send(Request::new("hello".to_string()))
            .await
            .expect("Failed to send message");
        assert_eq!(response.payload, "hello");
    }
}
