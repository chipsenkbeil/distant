use crate::{
    Codec, FramedTransport, IntoSplit, MappedListener, Server, ServerExt, WindowsPipeListener,
    WindowsPipeServerRef,
};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    ffi::{OsStr, OsString},
    io,
};

/// Extension trait to provide a reference implementation of starting a Windows pipe server
/// that will listen for new connections and process them using the [`Server`] implementation
#[async_trait]
pub trait WindowsPipeServerExt {
    type Request;
    type Response;

    /// Start a new server at the specified address using the given codec
    async fn start<A, C>(self, addr: A, codec: C) -> io::Result<WindowsPipeServerRef>
    where
        A: AsRef<OsStr> + Send,
        C: Codec + Send + Sync + 'static;

    /// Start a new server at the specified address via `\\.\pipe\{name}` using the given codec
    async fn start_local<N, C>(self, name: N, codec: C) -> io::Result<WindowsPipeServerRef>
    where
        Self: Sized,
        N: AsRef<OsStr> + Send,
        C: Codec + Send + Sync + 'static,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        self.start(addr, codec).await
    }
}

#[async_trait]
impl<S, Req, Res, Gdata, Ldata> WindowsPipeServerExt for S
where
    S: Server<Request = Req, Response = Res, GlobalData = Gdata, LocalData = Ldata>
        + Sync
        + 'static,
    Req: DeserializeOwned + Send + Sync,
    Res: Serialize + Send + 'static,
    Gdata: Default + Send + Sync + 'static,
    Ldata: Default + Send + Sync + 'static,
{
    type Request = Req;
    type Response = Res;

    async fn start<A, C>(self, addr: A, codec: C) -> io::Result<WindowsPipeServerRef>
    where
        A: AsRef<OsStr> + Send,
        C: Codec + Send + Sync + 'static,
    {
        let a = addr.as_ref();
        let listener = WindowsPipeListener::bind(a)?;
        let addr = listener.addr().to_os_string();

        let listener = MappedListener::new(listener, move |transport| {
            let transport = FramedTransport::new(transport, codec.clone());
            transport.into_split()
        });
        let inner = ServerExt::start(self, listener)?;
        Ok(WindowsPipeServerRef { addr, inner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Client, PlainCodec, Request, ServerCtx, WindowsPipeClientExt};

    pub struct TestServer;

    #[async_trait]
    impl Server for TestServer {
        type Request = String;
        type Response = String;
        type GlobalData = ();
        type LocalData = ();

        async fn on_request(
            &self,
            ctx: ServerCtx<Self::Request, Self::Response, Self::GlobalData, Self::LocalData>,
        ) {
            // Echo back what we received
            ctx.reply
                .send(ctx.request.payload.to_string())
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        let server = WindowsPipeServerExt::start_local(
            TestServer,
            format!("test_pip_{}", rand::random::<usize>()),
            PlainCodec,
        )
        .await
        .expect("Failed to start Windows pipe server");

        let mut client: Client<String, String> = Client::connect(server.addr(), PlainCodec)
            .await
            .expect("Client failed to connect");

        let response = client
            .send(Request::new("hello".to_string()))
            .await
            .expect("Failed to send message");
        assert_eq!(response.payload, "hello");
    }
}