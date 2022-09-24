use crate::{PortRange, Server, ServerExt, TcpListener, TcpServerRef};
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
    async fn start<P>(self, addr: IpAddr, port: P) -> io::Result<TcpServerRef>
    where
        P: Into<PortRange> + Send;
}

#[async_trait]
impl<S> TcpServerExt for S
where
    S: Server + Sync + 'static,
    S::Request: DeserializeOwned + Send + Sync + 'static,
    S::Response: Serialize + Send + 'static,
    S::LocalData: Default + Send + Sync + 'static,
{
    type Request = S::Request;
    type Response = S::Response;

    async fn start<P>(self, addr: IpAddr, port: P) -> io::Result<TcpServerRef>
    where
        P: Into<PortRange> + Send,
    {
        let listener = TcpListener::bind(addr, port).await?;
        let port = listener.port();
        let inner = ServerExt::start(self, listener)?;
        Ok(TcpServerRef { addr, port, inner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::{AuthHandler, Authenticator, Question, VerificationKind},
        Client, ConnectionCtx, Request, ServerCtx,
    };
    use std::{
        collections::HashMap,
        net::{Ipv6Addr, SocketAddr},
    };

    pub struct TestServer;

    #[async_trait]
    impl Server for TestServer {
        type Request = String;
        type Response = String;
        type LocalData = ();

        async fn on_accept<A: Authenticator>(
            &self,
            ctx: ConnectionCtx<'_, A, Self::LocalData>,
        ) -> io::Result<()> {
            ctx.authenticator.finished().await
        }

        async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
            // Echo back what we received
            ctx.reply
                .send(ctx.request.payload.to_string())
                .await
                .unwrap();
        }
    }

    pub struct TestAuthHandler;

    #[async_trait]
    impl AuthHandler for TestAuthHandler {
        async fn on_challenge(
            &mut self,
            _: Vec<Question>,
            _: HashMap<String, String>,
        ) -> io::Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn on_verify(&mut self, _: VerificationKind, _: String) -> io::Result<bool> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        let server = TcpServerExt::start(TestServer, IpAddr::V6(Ipv6Addr::LOCALHOST), 0)
            .await
            .expect("Failed to start TCP server");

        let mut client: Client<String, String> = Client::<String, String>::tcp()
            .auth_handler(TestAuthHandler)
            .connect(SocketAddr::from((server.ip_addr(), server.port())))
            .await
            .expect("Client failed to connect");

        let response = client
            .send(Request::new("hello".to_string()))
            .await
            .expect("Failed to send message");
        assert_eq!(response.payload, "hello");
    }
}
