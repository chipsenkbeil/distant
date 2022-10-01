use crate::{
    auth::Verifier, Server, ServerConfig, ServerHandler, UnixSocketListener, UnixSocketServerRef,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{io, path::Path};

pub struct UnixSocketServerBuilder<T>(Server<T>);

impl Default for UnixSocketServerBuilder<()> {
    fn default() -> Self {
        Self(Server::new())
    }
}

impl<T> UnixSocketServerBuilder<T> {
    pub fn config(self, config: ServerConfig) -> Self {
        Self(self.0.config(config))
    }

    pub fn handler<U>(self, handler: U) -> UnixSocketServerBuilder<U> {
        UnixSocketServerBuilder(self.0.handler(handler))
    }

    pub fn verifier(self, verifier: Verifier) -> Self {
        Self(self.0.verifier(verifier))
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
        let inner = self.0.start(listener)?;
        Ok(UnixSocketServerRef { path, inner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{auth::DummyAuthHandler, Client, Request, ServerCtx};
    use async_trait::async_trait;
    use tempfile::NamedTempFile;
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

    #[test(tokio::test)]
    async fn should_invoke_handler_upon_receiving_a_request() {
        // Generate a socket path and delete the file after so there is nothing there
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .path()
            .to_path_buf();

        let server = UnixSocketServerBuilder::default()
            .handler(TestServerHandler)
            .verifier(Verifier::none())
            .start(path)
            .await
            .expect("Failed to start Unix socket server");

        let mut client: Client<String, String> = Client::unix_socket()
            .auth_handler(DummyAuthHandler)
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
