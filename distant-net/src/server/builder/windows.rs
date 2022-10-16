use crate::common::{auth::Verifier, WindowsPipeListener};
use crate::server::{Server, ServerConfig, ServerHandler, WindowsPipeServerRef};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    ffi::{OsStr, OsString},
    io,
};

pub struct WindowsPipeServerBuilder<T>(Server<T>);

impl<T> Server<T> {
    /// Consume [`Server`] and produce a builder for a Windows pipe variant.
    pub fn into_windows_pipe_builder(self) -> WindowsPipeServerBuilder<T> {
        WindowsPipeServerBuilder(self)
    }
}

impl Default for WindowsPipeServerBuilder<()> {
    fn default() -> Self {
        Self(Server::new())
    }
}

impl<T> WindowsPipeServerBuilder<T> {
    pub fn config(self, config: ServerConfig) -> Self {
        Self(self.0.config(config))
    }

    pub fn handler<U>(self, handler: U) -> WindowsPipeServerBuilder<U> {
        WindowsPipeServerBuilder(self.0.handler(handler))
    }

    pub fn verifier(self, verifier: Verifier) -> Self {
        Self(self.0.verifier(verifier))
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
        let inner = self.0.start(listener)?;
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
    use crate::client::Client;
    use crate::common::{auth::DummyAuthHandler, Request};
    use crate::server::ServerCtx;
    use async_trait::async_trait;
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
        let server = WindowsPipeServerBuilder::default()
            .handler(TestServerHandler)
            .verifier(Verifier::none())
            .start_local(format!("test_pipe_{}", rand::random::<usize>()))
            .await
            .expect("Failed to start Windows pipe server");

        let mut client: Client<String, String> = Client::windows_pipe()
            .auth_handler(DummyAuthHandler)
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
