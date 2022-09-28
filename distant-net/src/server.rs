use crate::{auth::Authenticator, Listener, Transport};
use async_trait::async_trait;
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{io, sync::Arc};
use tokio::sync::RwLock;

mod config;
pub use config::*;

mod connection;
pub use connection::*;

mod context;
pub use context::*;

mod ext;
pub use ext::*;

mod r#ref;
pub use r#ref::*;

mod reply;
pub use reply::*;

mod state;
pub use state::*;

mod shutdown_timer;
pub use shutdown_timer::*;

/// Represents a server that can be used to receive requests & send responses to clients.
pub struct Server<T> {
    /// Custom configuration details associated with the server
    config: ServerConfig,

    /// Handler used to process various server events
    handler: T,
}

/// Interface for a handler that receives connections and requests
#[async_trait]
pub trait ServerHandler: Send {
    /// Type of data received by the server
    type Request;

    /// Type of data sent back by the server
    type Response;

    /// Type of data to store locally tied to the specific connection
    type LocalData;

    /// Invoked upon a new connection becoming established.
    ///
    /// ### Note
    ///
    /// This can be useful in performing some additional initialization on the connection's local
    /// data prior to it being used anywhere else.
    ///
    /// Additionally, the context contains an authenticator which can be used to issue challenges
    /// to the connection to validate its access.
    async fn on_accept<A: Authenticator>(
        &self,
        ctx: ConnectionCtx<'_, A, Self::LocalData>,
    ) -> io::Result<()>;

    /// Invoked upon receiving a request from a client. The server should process this
    /// request, which can be found in `ctx`, and send one or more replies in response.
    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>);
}

impl<T> Server<T>
where
    T: ServerHandler + Sync + 'static,
    T::Request: DeserializeOwned + Send + Sync + 'static,
    T::Response: Serialize + Send + 'static,
    T::LocalData: Default + Send + Sync + 'static,
{
    /// Consumes the server, starting a task to process connections from the `listener` and
    /// returning a [`ServerRef`] that can be used to control the active server instance.
    pub fn start<L>(self, listener: L) -> io::Result<Box<dyn ServerRef>>
    where
        L: Listener + 'static,
        L::Output: Transport + Send + Sync + 'static,
    {
        let state = Arc::new(ServerState::new());
        let task = tokio::spawn(self.task(Arc::clone(&state), listener));

        Ok(Box::new(GenericServerRef { state, task }))
    }

    /// Internal task that is run to receive connections and spawn connection tasks
    async fn task<L>(self, state: Arc<ServerState>, mut listener: L)
    where
        L: Listener + 'static,
        L::Output: Transport + Send + Sync + 'static,
    {
        let Server { config, handler } = self;

        let handler = Arc::new(handler);
        let timer = ShutdownTimer::new(config.shutdown);
        let mut notification = timer.clone_notification();
        let timer = Arc::new(RwLock::new(timer));

        loop {
            // Receive a new connection, exiting if no longer accepting connections or if the shutdown
            // signal has been received
            let transport = tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok(x) => x,
                        Err(x) => {
                            error!("Server no longer accepting connections: {x}");
                            timer.read().await.abort();
                            break;
                        }
                    }
                }
                _ = notification.wait() => {
                    info!(
                        "Server shutdown triggered after {}s",
                        config.shutdown.duration().unwrap_or_default().as_secs_f32(),
                    );
                    break;
                }
            };

            // Ensure that the shutdown timer is cancelled now that we have a connection
            timer.read().await.stop();

            let connection = Connection::build()
                .handler(Arc::downgrade(&handler))
                .state(Arc::downgrade(&state))
                .transport(transport)
                .shutdown_timer(Arc::downgrade(&timer))
                .spawn();

            state
                .connections
                .write()
                .await
                .insert(connection.id(), connection);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::Authenticator, InmemoryTransport, MpscListener, Request, Response, ServerConfig,
    };
    use async_trait::async_trait;
    use std::time::Duration;
    use tokio::sync::mpsc;

    pub struct TestServerHandler(ServerConfig);

    #[async_trait]
    impl ServerHandler for TestServerHandler {
        type Request = u16;
        type Response = String;
        type LocalData = ();

        async fn on_accept<A: Authenticator>(
            &self,
            ctx: ConnectionCtx<'_, A, Self::LocalData>,
        ) -> io::Result<()> {
            ctx.authenticator.finished().await
        }

        async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
            // Always send back "hello"
            ctx.reply.send("hello".to_string()).await.unwrap();
        }
    }

    #[allow(clippy::type_complexity)]
    fn make_listener(
        buffer: usize,
    ) -> (
        mpsc::Sender<InmemoryTransport>,
        MpscListener<InmemoryTransport>,
    ) {
        MpscListener::channel(buffer)
    }

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        // Create a test listener where we will forward a connection
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (transport, connection) = InmemoryTransport::pair(100);
        tx.send(connection)
            .await
            .expect("Failed to feed listener a connection");

        let _server = ServerExt::start(TestServerHandler(ServerConfig::default()), listener)
            .expect("Failed to start server");

        transport
            .try_write(&Request::new(123).to_vec().unwrap())
            .expect("Failed to send request");

        let mut buf = [0u8; 1024];
        let n = transport.try_read(&mut buf).unwrap();
        let response: Response<String> = Response::from_slice(&buf[..n]).unwrap();
        assert_eq!(response.payload, "hello");
    }

    #[tokio::test]
    async fn should_lonely_shutdown_if_no_connections_received_after_n_secs_when_config_set() {
        let (_tx, listener) = make_listener(100);

        let server = ServerExt::start(
            TestServerHandler(ServerConfig {
                shutdown: Shutdown::Lonely(Duration::from_millis(100)),
                ..Default::default()
            }),
            listener,
        )
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(server.is_finished(), "Server shutdown not triggered!");
    }

    #[tokio::test]
    async fn should_lonely_shutdown_if_last_connection_terminated_and_then_no_connections_after_n_secs(
    ) {
        // Create a test listener where we will forward a connection
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (transport, connection) = InmemoryTransport::pair(100);
        tx.send(connection)
            .await
            .expect("Failed to feed listener a connection");

        let server = ServerExt::start(
            TestServerHandler(ServerConfig {
                shutdown: Shutdown::Lonely(Duration::from_millis(100)),
                ..Default::default()
            }),
            listener,
        )
        .expect("Failed to start server");

        // Drop the connection by dropping the transport
        drop(transport);

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(server.is_finished(), "Server shutdown not triggered!");
    }

    #[tokio::test]
    async fn should_not_lonely_shutdown_as_long_as_a_connection_exists() {
        // Create a test listener where we will forward a connection
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (_transport, connection) = InmemoryTransport::pair(100);
        tx.send(connection)
            .await
            .expect("Failed to feed listener a connection");

        let server = ServerExt::start(
            TestServerHandler(ServerConfig {
                shutdown: Shutdown::Lonely(Duration::from_millis(100)),
                ..Default::default()
            }),
            listener,
        )
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(!server.is_finished(), "Server shutdown when it should not!");
    }

    #[tokio::test]
    async fn should_shutdown_after_n_seconds_even_with_connections_if_config_set_to_after() {
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (_transport, connection) = InmemoryTransport::pair(100);
        tx.send(connection)
            .await
            .expect("Failed to feed listener a connection");

        let server = ServerExt::start(
            TestServerHandler(ServerConfig {
                shutdown: Shutdown::After(Duration::from_millis(100)),
                ..Default::default()
            }),
            listener,
        )
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(server.is_finished(), "Server shutdown not triggered!");
    }

    #[tokio::test]
    async fn should_shutdown_after_n_seconds_if_config_set_to_after() {
        let (_tx, listener) = make_listener(100);

        let server = ServerExt::start(
            TestServerHandler(ServerConfig {
                shutdown: Shutdown::After(Duration::from_millis(100)),
                ..Default::default()
            }),
            listener,
        )
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(server.is_finished(), "Server shutdown not triggered!");
    }

    #[tokio::test]
    async fn should_never_shutdown_if_config_set_to_never() {
        let (_tx, listener) = make_listener(100);

        let server = ServerExt::start(
            TestServerHandler(ServerConfig {
                shutdown: Shutdown::Never,
                ..Default::default()
            }),
            listener,
        )
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(!server.is_finished(), "Server shutdown when it should not!");
    }
}
