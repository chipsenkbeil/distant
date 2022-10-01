use crate::{
    auth::Verifier, PortRange, Server, ServerConfig, ServerHandler, TcpListener, TcpServerRef,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{io, net::IpAddr};

pub struct TcpServerBuilder<T>(Server<T>);

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
        let inner = self.0.start(listener)?;
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
            AuthHandler,
        },
        Client, Request, ServerCtx,
    };
    use async_trait::async_trait;
    use std::net::{Ipv6Addr, SocketAddr};
    use test_log::test;

    pub struct TestServerHandler;

    #[async_trait]
    impl ServerHandler for TestServerHandler {
        type Request = String;
        type Response = String;
        type LocalData = ();

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

    #[test(tokio::test)]
    async fn should_invoke_handler_upon_receiving_a_request() {
        let server = TcpServerBuilder::default()
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
