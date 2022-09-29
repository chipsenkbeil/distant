use crate::{
    auth::Verifier, Server, ServerConfig, ServerHandler, WindowsPipeListener, WindowsPipeServerRef,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    ffi::{OsStr, OsString},
    io,
};

pub struct WindowsPipeServerBuilder<T> {
    config: ServerConfig,
    handler: T,
    verifier: Verifier,
}

impl Default for WindowsPipeServerBuilder<()> {
    fn default() -> Self {
        Self {
            config: Default::default(),
            handler: (),
            verifier: Verifier::empty(),
        }
    }
}

impl<T> WindowsPipeServerBuilder<T> {
    pub fn verifier(self, verifier: Verifier) -> Self {
        Self {
            config: self.config,
            handler: self.handler,
            verifier,
        }
    }

    pub fn config(self, config: ServerConfig) -> Self {
        Self {
            config,
            handler: self.handler,
            verifier: self.verifier,
        }
    }

    pub fn handler<U>(self, handler: U) -> WindowsPipeServerBuilder<U> {
        WindowsPipeServerBuilder {
            config: self.config,
            handler,
            verifier: self.verifier,
        }
    }
}

impl<T> WindowsPipeServerBuilder<T>
where
    T: ServerHandler + Sync + 'static,
    T::Request: DeserializeOwned + Send + Sync + 'static,
    T::Response: Serialize + Send + 'static,
    T::LocalData: Default + Send + Sync + 'static,
{
    /// Start a new server at the specified address using the given codec
    pub async fn start<A>(self, addr: A) -> io::Result<WindowsPipeServerRef>
    where
        A: AsRef<OsStr> + Send,
    {
        let a = addr.as_ref();
        let listener = WindowsPipeListener::bind(a)?;
        let addr = listener.addr().to_os_string();

        let server = Server {
            config: self.config,
            handler: self.handler,
            verifier: self.verifier,
        };
        let inner = server.start(listener)?;
        Ok(WindowsPipeServerRef { addr, inner })
    }

    /// Start a new server at the specified address via `\\.\pipe\{name}` using the given codec
    pub async fn start_local<N>(self, name: N) -> io::Result<WindowsPipeServerRef>
    where
        Self: Sized,
        N: AsRef<OsStr> + Send,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        self.start(addr).await
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
    use test_log::test;

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

    #[test(tokio::test)]
    async fn should_invoke_handler_upon_receiving_a_request() {
        let server = Server::windows_pipe()
            .handler(TestServerHandler)
            .start_local(format!("test_pipe_{}", rand::random::<usize>()))
            .await
            .expect("Failed to start Windows pipe server");

        let mut client: Client<String, String> = Client::windows_pipe()
            .auth_handler(TestAuthHandler)
            .connect(server.addr())
            .await
            .expect("Client failed to connect");

        let response = client
            .send(Request::new("hello".to_string()))
            .await
            .expect("Failed to send message");
        assert_eq!(response.payload, "hello");
    }
}
