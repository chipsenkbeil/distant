use crate::{
    auth::Verifier, Server, ServerConfig, ServerHandler, UnixSocketListener, UnixSocketServerRef,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{io, path::Path};

pub struct UnixSocketServerBuilder<T> {
    config: ServerConfig,
    handler: T,
    verifier: Verifier,
}

impl Default for UnixSocketServerBuilder<()> {
    fn default() -> Self {
        Self {
            config: Default::default(),
            handler: (),
            verifier: Verifier::empty(),
        }
    }
}

impl<T> UnixSocketServerBuilder<T> {
    pub fn config(self, config: ServerConfig) -> Self {
        Self {
            config,
            handler: self.handler,
            verifier: self.verifier,
        }
    }

    pub fn handler<U>(self, handler: U) -> UnixSocketServerBuilder<U> {
        UnixSocketServerBuilder {
            config: self.config,
            handler,
            verifier: self.verifier,
        }
    }

    pub fn verifier(self, verifier: Verifier) -> Self {
        Self {
            config: self.config,
            handler: self.handler,
            verifier,
        }
    }
}

impl<T> UnixSocketServerBuilder<T>
where
    T: ServerHandler + Sync + 'static,
    T::Request: DeserializeOwned + Send + Sync + 'static,
    T::Response: Serialize + Send + 'static,
    T::LocalData: Default + Send + Sync + 'static,
{
    pub async fn start<P>(self, path: P) -> io::Result<UnixSocketServerRef>
    where
        P: AsRef<Path> + Send,
    {
        let path = path.as_ref();
        let listener = UnixSocketListener::bind(path).await?;
        let path = listener.path().to_path_buf();

        let server = Server {
            config: self.config,
            handler: self.handler,
            verifier: self.verifier,
        };
        let inner = server.start(listener)?;
        Ok(UnixSocketServerRef { path, inner })
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
    use tempfile::NamedTempFile;
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
        // Generate a socket path and delete the file after so there is nothing there
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .path()
            .to_path_buf();

        let server = Server::unix_socket()
            .handler(TestServerHandler)
            .start(path)
            .await
            .expect("Failed to start Unix socket server");

        let mut client: Client<String, String> = Client::unix_socket()
            .auth_handler(TestAuthHandler)
            .connect(server.path())
            .await
            .expect("Client failed to connect");

        let response = client
            .send(Request::new("hello".to_string()))
            .await
            .expect("Failed to send message");
        assert_eq!(response.payload, "hello");
    }
}
