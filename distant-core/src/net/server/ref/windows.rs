use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::task::JoinError;

use super::ServerRef;

/// Reference to a windows pipe server instance.
pub struct WindowsPipeServerRef {
    pub(crate) addr: OsString,
    pub(crate) inner: ServerRef,
}

impl WindowsPipeServerRef {
    pub fn new(addr: OsString, inner: ServerRef) -> Self {
        Self { addr, inner }
    }

    /// Returns the addr that the listener is bound to.
    pub fn addr(&self) -> &OsStr {
        &self.addr
    }

    /// Consumes ref, returning inner ref.
    pub fn into_inner(self) -> ServerRef {
        self.inner
    }
}

impl Future for WindowsPipeServerRef {
    type Output = Result<(), JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner.task).poll(cx)
    }
}

impl Deref for WindowsPipeServerRef {
    type Target = ServerRef;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for WindowsPipeServerRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    //! Tests for WindowsPipeServerRef: construction, addr accessor, into_inner,
    //! Deref/DerefMut, and shutdown. The `make_server_ref` tests use a no-op task
    //! for testing accessor wiring. The `shutdown_stops_real_server` test uses a
    //! real `Server` instance to verify that shutdown actually stops an active server.

    use super::*;
    use std::ffi::OsString;
    use tokio::sync::broadcast;

    fn make_server_ref() -> ServerRef {
        let (shutdown, _) = broadcast::channel(1);
        let task = tokio::spawn(async {});
        ServerRef { shutdown, task }
    }

    #[test_log::test(tokio::test)]
    async fn new_stores_addr() {
        let addr = OsString::from(r"\\.\pipe\test_pipe");
        let pipe_ref = WindowsPipeServerRef::new(addr.clone(), make_server_ref());
        assert_eq!(pipe_ref.addr(), addr.as_os_str());
    }

    #[test_log::test(tokio::test)]
    async fn addr_returns_correct_value() {
        let addr = OsString::from(r"\\.\pipe\another_test_pipe");
        let pipe_ref = WindowsPipeServerRef::new(addr, make_server_ref());
        assert_eq!(pipe_ref.addr(), OsStr::new(r"\\.\pipe\another_test_pipe"));
    }

    #[test_log::test(tokio::test)]
    async fn into_inner_returns_server_ref() {
        let addr = OsString::from(r"\\.\pipe\test_pipe");
        let pipe_ref = WindowsPipeServerRef::new(addr, make_server_ref());
        let recovered = pipe_ref.into_inner();
        // Let the spawned empty task complete
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(recovered.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn deref_delegates_to_inner_server_ref() {
        let addr = OsString::from(r"\\.\pipe\test_pipe");
        let pipe_ref = WindowsPipeServerRef::new(addr, make_server_ref());
        // Deref gives us access to ServerRef methods
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(pipe_ref.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn deref_mut_delegates_to_inner_server_ref() {
        let addr = OsString::from(r"\\.\pipe\test_pipe");
        let mut pipe_ref = WindowsPipeServerRef::new(addr, make_server_ref());
        let inner: &mut ServerRef = &mut pipe_ref;
        inner.shutdown();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(pipe_ref.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn shutdown_via_deref_stops_server() {
        let addr = OsString::from(r"\\.\pipe\test_pipe");
        let pipe_ref = WindowsPipeServerRef::new(addr, make_server_ref());
        pipe_ref.shutdown();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(pipe_ref.is_finished());
    }

    // --- Real server shutdown test ---

    use crate::auth::{AuthenticationMethod, NoneAuthenticationMethod, Verifier};
    use crate::net::common::{InmemoryTransport, MpscListener, Version};
    use crate::net::server::{RequestCtx, Server, ServerConfig, ServerHandler};

    struct TestServerHandler;

    impl ServerHandler for TestServerHandler {
        type Request = u16;
        type Response = String;

        async fn on_request(&self, ctx: RequestCtx<Self::Request, Self::Response>) {
            ctx.reply.send("hello".to_string()).unwrap();
        }
    }

    fn start_real_server() -> (
        WindowsPipeServerRef,
        tokio::sync::mpsc::Sender<InmemoryTransport>,
    ) {
        let (tx, listener) = MpscListener::channel(100);
        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(NoneAuthenticationMethod::new())];
        let server_ref = Server {
            config: ServerConfig::default(),
            handler: TestServerHandler,
            verifier: Verifier::new(methods),
            version: Version::new(1, 2, 3),
        }
        .start(listener)
        .expect("Failed to start server");
        (
            WindowsPipeServerRef::new(OsString::from(r"\\.\pipe\test_pipe"), server_ref),
            tx,
        )
    }

    #[test_log::test(tokio::test)]
    async fn shutdown_stops_real_server() {
        let (server_ref, listener_tx) = start_real_server();
        assert!(!server_ref.is_finished());
        server_ref.shutdown();
        drop(listener_tx); // Close listener so accept loop exits
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(server_ref.is_finished());
    }
}
