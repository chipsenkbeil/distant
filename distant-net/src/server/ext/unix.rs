use crate::{
    Codec, FramedTransport, IntoSplit, MappedListener, Server, ServerExt, UnixSocketListener,
    UnixSocketServerRef,
};
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
    async fn start<P, C>(self, path: P, codec: C) -> io::Result<UnixSocketServerRef>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + Sync + 'static;
}

#[async_trait]
impl<S, Req, Res, Data> UnixSocketServerExt for S
where
    S: Server<Request = Req, Response = Res, LocalData = Data> + Sync + 'static,
    Req: DeserializeOwned + Send + Sync + 'static,
    Res: Serialize + Send + 'static,
    Data: Default + Send + Sync + 'static,
{
    type Request = Req;
    type Response = Res;

    async fn start<P, C>(self, path: P, codec: C) -> io::Result<UnixSocketServerRef>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + Sync + 'static,
    {
        let path = path.as_ref();
        let listener = UnixSocketListener::bind(path).await?;
        let path = listener.path().to_path_buf();

        let listener = MappedListener::new(listener, move |transport| {
            let transport = FramedTransport::new(transport, codec.clone());
            transport.into_split()
        });
        let inner = ServerExt::start(self, listener)?;
        Ok(UnixSocketServerRef { path, inner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Client, PlainCodec, Request, ServerCtx, UnixSocketClientExt};
    use tempfile::NamedTempFile;

    pub struct TestServer;

    #[async_trait]
    impl Server for TestServer {
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

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        // Generate a socket path and delete the file after so there is nothing there
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .path()
            .to_path_buf();

        let server = UnixSocketServerExt::start(TestServer, path, PlainCodec)
            .await
            .expect("Failed to start Unix socket server");

        let mut client: Client<String, String> = Client::connect(server.path(), PlainCodec)
            .await
            .expect("Client failed to connect");

        let response = client
            .send(Request::new("hello".to_string()))
            .await
            .expect("Failed to send message");
        assert_eq!(response.payload, "hello");
    }
}
