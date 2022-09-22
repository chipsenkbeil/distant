use crate::{Server, ServerExt, WindowsPipeListener, WindowsPipeServerRef};
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
    async fn start<A>(self, addr: A) -> io::Result<WindowsPipeServerRef>
    where
        A: AsRef<OsStr> + Send;

    /// Start a new server at the specified address via `\\.\pipe\{name}` using the given codec
    async fn start_local<N>(self, name: N) -> io::Result<WindowsPipeServerRef>
    where
        Self: Sized,
        N: AsRef<OsStr> + Send,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        self.start(addr).await
    }
}

#[async_trait]
impl<S> WindowsPipeServerExt for S
where
    S: Server + Sync + 'static,
    S::Request: DeserializeOwned + Send + Sync + 'static,
    S::Response: Serialize + Send + 'static,
    S::LocalData: Default + Send + Sync + 'static,
{
    type Request = S::Request;
    type Response = S::Response;

    async fn start<A>(self, addr: A) -> io::Result<WindowsPipeServerRef>
    where
        A: AsRef<OsStr> + Send,
    {
        let a = addr.as_ref();
        let listener = WindowsPipeListener::bind(a)?;
        let addr = listener.addr().to_os_string();

        let inner = ServerExt::start(self, listener)?;
        Ok(WindowsPipeServerRef { addr, inner })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::{AuthHandler, AuthQuestion, AuthVerifyKind, Authenticator},
        Client, Client, ConnectionCtx, ConnectionCtx, Request, Request, ServerCtx, ServerCtx,
    };
    use std::collections::HashMap;

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
        let server = WindowsPipeServerExt::start_local(
            TestServer,
            format!("test_pip_{}", rand::random::<usize>()),
        )
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
