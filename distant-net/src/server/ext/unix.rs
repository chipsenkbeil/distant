use crate::{Server, ServerExt, UnixSocketListener, UnixSocketServerRef};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::{io, path::Path};

/// Extension trait to provide a reference implementation of starting a Unix socket server
/// that will listen for new connections and process them using the [`Server`] implementation
#[async_trait]
pub trait UnixSocketServerExt {
    type Request;
    type Response;

    /// Start a new server using the provided listener
    async fn start<P>(self, path: P) -> io::Result<UnixSocketServerRef>
    where
        P: AsRef<Path> + Send;
}

#[async_trait]
impl<S> UnixSocketServerExt for S
where
    S: Server + Sync + 'static,
    S::Request: DeserializeOwned + Send + Sync + 'static,
    S::Response: Serialize + Send + 'static,
    S::LocalData: Default + Send + Sync + 'static,
{
    type Request = S::Request;
    type Response = S::Response;

    async fn start<P>(self, path: P) -> io::Result<UnixSocketServerRef>
    where
        P: AsRef<Path> + Send,
    {
        let path = path.as_ref();
        let listener = UnixSocketListener::bind(path).await?;
        let path = listener.path().to_path_buf();

        let inner = ServerExt::start(self, listener)?;
        Ok(UnixSocketServerRef { path, inner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::{AuthHandler, AuthQuestion, AuthVerifyKind, Authenticator},
        Client, ConnectionCtx, Request, ServerCtx,
    };
    use std::collections::HashMap;
    use tempfile::NamedTempFile;

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
            _: Vec<AuthQuestion>,
            _: HashMap<String, String>,
        ) -> io::Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn on_verify(&mut self, _: AuthVerifyKind, _: String) -> io::Result<bool> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        // Generate a socket path and delete the file after so there is nothing there
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .path()
            .to_path_buf();

        let server = UnixSocketServerExt::start(TestServer, path)
            .await
            .expect("Failed to start Unix socket server");

        let mut client: Client<String, String> = Client::<String, String>::unix_socket()
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
