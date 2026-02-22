use std::io;
use std::net::IpAddr;

use crate::auth::Verifier;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::net::common::{PortRange, TcpListener, Version};
use crate::net::server::{Server, ServerConfig, ServerHandler, TcpServerRef};

pub struct TcpServerBuilder<T>(Server<T>);

impl<T> Server<T> {
    /// Consume [`Server`] and produce a builder for a TCP variant.
    pub fn into_tcp_builder(self) -> TcpServerBuilder<T> {
        TcpServerBuilder(self)
    }
}

impl Default for TcpServerBuilder<()> {
    fn default() -> Self {
        Self(Server::new())
    }
}

impl<T> TcpServerBuilder<T> {
    pub fn config(self, config: ServerConfig) -> Self {
        Self(self.0.config(config))
    }

    pub fn handler<U>(self, handler: U) -> TcpServerBuilder<U> {
        TcpServerBuilder(self.0.handler(handler))
    }

    pub fn verifier(self, verifier: Verifier) -> Self {
        Self(self.0.verifier(verifier))
    }

    pub fn version(self, version: Version) -> Self {
        Self(self.0.version(version))
    }
}

impl<T> TcpServerBuilder<T>
where
    T: ServerHandler + Sync + 'static,
    T::Request: DeserializeOwned + Send + Sync + 'static,
    T::Response: Serialize + Send + 'static,
{
    pub async fn start<P>(self, addr: IpAddr, port: P) -> io::Result<TcpServerRef>
    where
        P: Into<PortRange> + Send,
    {
        let listener = TcpListener::bind(addr, port).await?;
        let port = listener.port();
        let inner = self.0.start(listener)?;
        Ok(TcpServerRef { addr, port, inner })
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv6Addr, SocketAddr};

    use crate::auth::DummyAuthHandler;
    use test_log::test;

    use super::*;
    use crate::net::client::Client;
    use crate::net::common::Request;
    use crate::net::server::RequestCtx;

    pub struct TestServerHandler;

    impl ServerHandler for TestServerHandler {
        type Request = String;
        type Response = String;

        async fn on_request(&self, ctx: RequestCtx<Self::Request, Self::Response>) {
            // Echo back what we received
            ctx.reply.send(ctx.request.payload.to_string()).unwrap();
        }
    }

    #[test(tokio::test)]
    async fn should_invoke_handler_upon_receiving_a_request() {
        let server = TcpServerBuilder::default()
            .handler(TestServerHandler)
            .verifier(Verifier::none())
            .start(IpAddr::V6(Ipv6Addr::LOCALHOST), 0)
            .await
            .expect("Failed to start TCP server");

        let mut client: Client<String, String> =
            Client::tcp(SocketAddr::from((server.ip_addr(), server.port())))
                .auth_handler(DummyAuthHandler)
                .connect()
                .await
                .expect("Client failed to connect");

        let response = client
            .send(Request::new("hello".to_string()))
            .await
            .expect("Failed to send message");
        assert_eq!(response.payload, "hello");
    }
}
