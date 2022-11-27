use crate::common::{authentication::Verifier, Listener, Transport};
use async_trait::async_trait;
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{io, sync::Arc, time::Duration};
use tokio::sync::RwLock;

mod builder;
pub use builder::*;

mod config;
pub use config::*;

mod connection;
use connection::*;

mod context;
pub use context::*;

mod r#ref;
pub use r#ref::*;

mod reply;
pub use reply::*;

mod state;
use state::*;

mod shutdown_timer;
use shutdown_timer::*;

/// Represents a server that can be used to receive requests & send responses to clients.
pub struct Server<T> {
    /// Custom configuration details associated with the server
    config: ServerConfig,

    /// Handler used to process various server events
    handler: T,

    /// Performs authentication using various methods
    verifier: Verifier,
}

/// Interface for a handler that receives connections and requests
#[async_trait]
pub trait ServerHandler: Send {
    /// Type of data received by the server
    type Request;

    /// Type of data sent back by the server
    type Response;

    /// Type of data to store locally tied to the specific connection
    type LocalData: Send;

    /// Invoked upon a new connection becoming established.
    ///
    /// ### Note
    ///
    /// This can be useful in performing some additional initialization on the connection's local
    /// data prior to it being used anywhere else.
    #[allow(unused_variables)]
    async fn on_accept(&self, ctx: ConnectionCtx<'_, Self::LocalData>) -> io::Result<()> {
        Ok(())
    }

    /// Invoked upon receiving a request from a client. The server should process this
    /// request, which can be found in `ctx`, and send one or more replies in response.
    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>);
}

impl Server<()> {
    /// Creates a new [`Server`], starting with a default configuration, no authentication methods,
    /// and no [`ServerHandler`].
    pub fn new() -> Self {
        Self {
            config: Default::default(),
            handler: (),
            verifier: Verifier::empty(),
        }
    }

    /// Creates a new [`TcpServerBuilder`] that is used to construct a [`Server`].
    pub fn tcp() -> TcpServerBuilder<()> {
        TcpServerBuilder::default()
    }

    /// Creates a new [`UnixSocketServerBuilder`] that is used to construct a [`Server`].
    #[cfg(unix)]
    pub fn unix_socket() -> UnixSocketServerBuilder<()> {
        UnixSocketServerBuilder::default()
    }

    /// Creates a new [`WindowsPipeServerBuilder`] that is used to construct a [`Server`].
    #[cfg(windows)]
    pub fn windows_pipe() -> WindowsPipeServerBuilder<()> {
        WindowsPipeServerBuilder::default()
    }
}

impl Default for Server<()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Server<T> {
    /// Consumes the current server, replacing its config with `config` and returning it.
    pub fn config(self, config: ServerConfig) -> Self {
        Self {
            config,
            handler: self.handler,
            verifier: self.verifier,
        }
    }

    /// Consumes the current server, replacing its handler with `handler` and returning it.
    pub fn handler<U>(self, handler: U) -> Server<U> {
        Server {
            config: self.config,
            handler,
            verifier: self.verifier,
        }
    }

    /// Consumes the current server, replacing its verifier with `verifier` and returning it.
    pub fn verifier(self, verifier: Verifier) -> Self {
        Self {
            config: self.config,
            handler: self.handler,
            verifier,
        }
    }
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
        L::Output: Transport + 'static,
    {
        let state = Arc::new(ServerState::new());
        let task = tokio::spawn(self.task(Arc::clone(&state), listener));

        Ok(Box::new(GenericServerRef { state, task }))
    }

    /// Internal task that is run to receive connections and spawn connection tasks
    async fn task<L>(self, state: Arc<ServerState>, mut listener: L)
    where
        L: Listener + 'static,
        L::Output: Transport + 'static,
    {
        let Server {
            config,
            handler,
            verifier,
        } = self;

        let handler = Arc::new(handler);
        let timer = ShutdownTimer::start(config.shutdown);
        let mut notification = timer.clone_notification();
        let timer = Arc::new(RwLock::new(timer));
        let verifier = Arc::new(verifier);

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

                    for (id, mut task) in state.connections.write().await.drain() {
                        info!("Shutting down connection {id}");
                        task.shutdown();
                        let _ = task.await;
                        debug!("Connection {id} shut down");
                    }

                    break;
                }
            };

            // Ensure that the shutdown timer is cancelled now that we have a connection
            timer.read().await.stop();

            let connection = ConnectionTask::build()
                .handler(Arc::downgrade(&handler))
                .state(Arc::downgrade(&state))
                .keychain(state.keychain.clone())
                .transport(transport)
                .shutdown_timer(Arc::downgrade(&timer))
                .sleep_duration(config.connection_sleep)
                .heartbeat_duration(config.connection_heartbeat)
                .verifier(Arc::downgrade(&verifier))
                .spawn();

            state
                .connections
                .write()
                .await
                .insert(connection.id(), connection);
        }

        // Once we stop listening, we still want to wait until all connections have terminated
        info!("Server waiting for active connections to terminate");
        while state.has_active_connections().await {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        info!("Server task terminated");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{
        authentication::{AuthenticationMethod, DummyAuthHandler, NoneAuthenticationMethod},
        Connection, InmemoryTransport, MpscListener, Request, Response,
    };
    use async_trait::async_trait;
    use std::time::Duration;
    use test_log::test;
    use tokio::sync::mpsc;

    pub struct TestServerHandler;

    #[async_trait]
    impl ServerHandler for TestServerHandler {
        type Request = u16;
        type Response = String;
        type LocalData = ();

        async fn on_accept(&self, _: ConnectionCtx<'_, Self::LocalData>) -> io::Result<()> {
            Ok(())
        }

        async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
            // Always send back "hello"
            ctx.reply.send("hello".to_string()).await.unwrap();
        }
    }

    #[inline]
    fn make_test_server(config: ServerConfig) -> Server<TestServerHandler> {
        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(NoneAuthenticationMethod::new())];

        Server {
            config,
            handler: TestServerHandler,
            verifier: Verifier::new(methods),
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

    #[test(tokio::test)]
    async fn should_invoke_handler_upon_receiving_a_request() {
        // Create a test listener where we will forward a connection
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (transport, connection) = InmemoryTransport::pair(100);
        tx.send(connection)
            .await
            .expect("Failed to feed listener a connection");

        let _server = make_test_server(ServerConfig::default())
            .start(listener)
            .expect("Failed to start server");

        // Perform handshake and authentication with the server before beginning to send data
        let mut connection = Connection::client(transport, DummyAuthHandler)
            .await
            .expect("Failed to connect to server");

        connection
            .write_frame(Request::new(123).to_vec().unwrap())
            .await
            .expect("Failed to send request");

        // Wait for a response
        let frame = connection.read_frame().await.unwrap().unwrap();
        let response: Response<String> = Response::from_slice(frame.as_item()).unwrap();
        assert_eq!(response.payload, "hello");
    }

    #[test(tokio::test)]
    async fn should_lonely_shutdown_if_no_connections_received_after_n_secs_when_config_set() {
        let (_tx, listener) = make_listener(100);

        let server = make_test_server(ServerConfig {
            shutdown: Shutdown::Lonely(Duration::from_millis(100)),
            ..Default::default()
        })
        .start(listener)
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(server.is_finished(), "Server shutdown not triggered!");
    }

    #[test(tokio::test)]
    async fn should_lonely_shutdown_if_last_connection_terminated_and_then_no_connections_after_n_secs(
    ) {
        // Create a test listener where we will forward a connection
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (transport, connection) = InmemoryTransport::pair(100);
        tx.send(connection)
            .await
            .expect("Failed to feed listener a connection");

        let server = make_test_server(ServerConfig {
            shutdown: Shutdown::Lonely(Duration::from_millis(100)),
            ..Default::default()
        })
        .start(listener)
        .expect("Failed to start server");

        // Drop the connection by dropping the transport
        drop(transport);

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(server.is_finished(), "Server shutdown not triggered!");
    }

    #[test(tokio::test)]
    async fn should_not_lonely_shutdown_as_long_as_a_connection_exists() {
        // Create a test listener where we will forward a connection
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (_transport, connection) = InmemoryTransport::pair(100);
        tx.send(connection)
            .await
            .expect("Failed to feed listener a connection");

        let server = make_test_server(ServerConfig {
            shutdown: Shutdown::Lonely(Duration::from_millis(100)),
            ..Default::default()
        })
        .start(listener)
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(!server.is_finished(), "Server shutdown when it should not!");
    }

    #[test(tokio::test)]
    async fn should_shutdown_after_n_seconds_even_with_connections_if_config_set_to_after() {
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (_transport, connection) = InmemoryTransport::pair(100);
        tx.send(connection)
            .await
            .expect("Failed to feed listener a connection");

        let server = make_test_server(ServerConfig {
            shutdown: Shutdown::After(Duration::from_millis(100)),
            ..Default::default()
        })
        .start(listener)
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(server.is_finished(), "Server shutdown not triggered!");
    }

    #[test(tokio::test)]
    async fn should_shutdown_after_n_seconds_if_config_set_to_after() {
        let (_tx, listener) = make_listener(100);

        let server = make_test_server(ServerConfig {
            shutdown: Shutdown::After(Duration::from_millis(100)),
            ..Default::default()
        })
        .start(listener)
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(server.is_finished(), "Server shutdown not triggered!");
    }

    #[test(tokio::test)]
    async fn should_never_shutdown_if_config_set_to_never() {
        let (_tx, listener) = make_listener(100);

        let server = make_test_server(ServerConfig {
            shutdown: Shutdown::Never,
            ..Default::default()
        })
        .start(listener)
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(!server.is_finished(), "Server shutdown when it should not!");
    }
}
