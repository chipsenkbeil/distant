use crate::{PortRange, Server, ServerConfig, ServerHandler, TcpListener, TcpServerRef};
use serde::{de::DeserializeOwned, Serialize};
use std::{io, net::IpAddr};

pub struct TcpServerBuilder<T> {
    config: ServerConfig,
    handler: T,
}

impl Default for TcpServerBuilder<()> {
    fn default() -> Self {
        Self {
            config: Default::default(),
            handler: (),
        }
    }
}

impl<T> TcpServerBuilder<T> {
    pub fn config(self, config: ServerConfig) -> Self {
        Self {
            config,
            handler: self.handler,
        }
    }

    pub fn handler<U>(self, handler: U) -> TcpServerBuilder<U> {
        TcpServerBuilder {
            config: self.config,
            handler,
        }
    }
}

impl<T> TcpServerBuilder<T>
where
    T: ServerHandler + Sync + 'static,
    T::Request: DeserializeOwned + Send + Sync + 'static,
    T::Response: Serialize + Send + 'static,
    T::LocalData: Default + Send + Sync + 'static,
{
    pub async fn start<P>(self, addr: IpAddr, port: P) -> io::Result<TcpServerRef>
    where
        P: Into<PortRange> + Send,
    {
        let listener = TcpListener::bind(addr, port).await?;
        let port = listener.port();
        let server = Server {
            config: self.config,
            handler: self.handler,
        };
        let inner = server.start(listener)?;
        Ok(TcpServerRef { addr, port, inner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::{
            msg::{
                Challenge, ChallengeResponse, Initialization, InitializationResponse, Verification,
                VerificationResponse,
            },
            AuthHandler, Authenticator,
        },
        Client, ConnectionCtx, Request, ServerCtx,
    };
    use async_trait::async_trait;
    use std::net::{Ipv6Addr, SocketAddr};

    pub struct TestServerHandler;

    #[async_trait]
    impl ServerHandler for TestServerHandler {
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
        async fn on_initialization(
            &mut self,
            x: Initialization,
        ) -> io::Result<InitializationResponse> {
            Ok(InitializationResponse { methods: x.methods })
        }

        async fn on_challenge(&mut self, _: Challenge) -> io::Result<ChallengeResponse> {
            Ok(ChallengeResponse {
                answers: Vec::new(),
            })
        }

        async fn on_verification(&mut self, _: Verification) -> io::Result<VerificationResponse> {
            Ok(VerificationResponse { valid: true })
        }
    }

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        let server = Server::tcp()
            .handler(TestServerHandler)
            .start(IpAddr::V6(Ipv6Addr::LOCALHOST), 0)
            .await
            .expect("Failed to start TCP server");

        let mut client: Client<String, String> = Client::tcp()
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
