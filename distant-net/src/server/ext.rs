use crate::{
    auth::FramedAuthenticator, utils::Timer, ConnectionCtx, ConnectionId, FramedTransport,
    GenericServerRef, Interest, Listener, Response, Server, ServerConnection, ServerCtx, ServerRef,
    ServerReply, ServerState, Shutdown, Transport, UntypedRequest,
};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io,
    sync::{Arc, Weak},
};
use tokio::{
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

mod tcp;
pub use tcp::*;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::*;

/// Extension trait to provide a reference implementation of starting a server
/// that will listen for new connections (exposed as [`TypedAsyncWrite`] and [`TypedAsyncRead`])
/// and process them using the [`Server`] implementation
pub trait ServerExt {
    type Request;
    type Response;

    /// Start a new server using the provided listener
    fn start<L>(self, listener: L) -> io::Result<Box<dyn ServerRef>>
    where
        L: Listener + 'static,
        L::Output: Transport + Send + Sync + 'static;
}

impl<S> ServerExt for S
where
    S: Server + Sync + 'static,
    S::Request: DeserializeOwned + Send + Sync + 'static,
    S::Response: Serialize + Send + 'static,
    S::LocalData: Default + Send + Sync + 'static,
{
    type Request = S::Request;
    type Response = S::Response;

    fn start<L>(self, listener: L) -> io::Result<Box<dyn ServerRef>>
    where
        L: Listener + 'static,
        L::Output: Transport + Send + Sync + 'static,
    {
        let server = Arc::new(self);
        let state = Arc::new(ServerState::new());

        let task = tokio::spawn(task(server, Arc::clone(&state), listener));

        Ok(Box::new(GenericServerRef { state, task }))
    }
}

async fn task<S, L>(server: Arc<S>, state: Arc<ServerState>, mut listener: L)
where
    S: Server + Sync + 'static,
    S::Request: DeserializeOwned + Send + Sync + 'static,
    S::Response: Serialize + Send + 'static,
    S::LocalData: Default + Send + Sync + 'static,
    L: Listener + 'static,
    L::Output: Transport + Send + Sync + 'static,
{
    // Grab a copy of our server's configuration so we can leverage it below
    let config = server.config();

    // Create the timer that will be used shutdown the server after duration elapsed
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

    // NOTE: We do a manual map such that the shutdown sender is not captured and dropped when
    //       there is no shutdown after configured. This is because we need the future for the
    //       shutdown receiver to last forever in the event that there is no shutdown configured,
    //       not return immediately, which is what would happen if the sender was dropped.
    #[allow(clippy::manual_map)]
    let mut shutdown_timer = match config.shutdown {
        // Create a timer, start it, and drop it so it will always happen
        Shutdown::After(duration) => {
            Timer::new(duration, async move {
                let _ = shutdown_tx.send(()).await;
            })
            .start();
            None
        }
        Shutdown::Lonely(duration) => Some(Timer::new(duration, async move {
            let _ = shutdown_tx.send(()).await;
        })),
        Shutdown::Never => None,
    };

    if let Some(timer) = shutdown_timer.as_mut() {
        info!(
            "Server shutdown timer configured: {}s",
            timer.duration().as_secs_f32()
        );
        timer.start();
    }

    let mut shutdown_timer = shutdown_timer.map(|timer| Arc::new(Mutex::new(timer)));

    loop {
        let server = Arc::clone(&server);

        // Receive a new connection, exiting if no longer accepting connections or if the shutdown
        // signal has been received
        let transport = tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok(x) => x,
                    Err(x) => {
                        error!("Server no longer accepting connections: {x}");
                        if let Some(timer) = shutdown_timer.take() {
                            timer.lock().await.abort();
                        }
                        break;
                    }
                }
            }
            _ = shutdown_rx.recv() => {
                info!(
                    "Server shutdown triggered after {}s",
                    config.shutdown.duration().unwrap_or_default().as_secs_f32(),
                );
                break;
            }
        };

        let mut connection = ServerConnection::new();
        let connection_id = connection.id;
        let state = Arc::clone(&state);

        // Ensure that the shutdown timer is cancelled now that we have a connection
        if let Some(timer) = shutdown_timer.as_ref() {
            timer.lock().await.stop();
        }

        connection.task = Some(
            ConnectionTask {
                id: connection_id,
                server,
                state: Arc::downgrade(&state),
                transport,
                shutdown_timer: shutdown_timer
                    .as_ref()
                    .map(Arc::downgrade)
                    .unwrap_or_default(),
            }
            .spawn(),
        );

        state
            .connections
            .write()
            .await
            .insert(connection_id, connection);
    }
}

struct ConnectionTask<S, T> {
    id: ConnectionId,
    server: Arc<S>,
    state: Weak<ServerState>,
    transport: T,
    shutdown_timer: Weak<Mutex<Timer<()>>>,
}

impl<S, T> ConnectionTask<S, T>
where
    S: Server + Sync + 'static,
    S::Request: DeserializeOwned + Send + Sync + 'static,
    S::Response: Serialize + Send + 'static,
    S::LocalData: Default + Send + Sync + 'static,
    T: Transport + Send + Sync + 'static,
{
    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(self.run())
    }

    async fn run(self) {
        let connection_id = self.id;

        // Construct a queue of outgoing responses
        let (tx, mut rx) = mpsc::channel::<Response<S::Response>>(1);

        // Perform a handshake to ensure that the connection is properly established
        let mut transport: FramedTransport<T> = FramedTransport::plain(self.transport);
        if let Err(x) = transport.server_handshake().await {
            error!("[Conn {connection_id}] Handshake failed: {x}");
            return;
        }

        // Create local data for the connection and then process it as well as perform
        // authentication and any other tasks on first connecting
        let mut local_data = S::LocalData::default();
        if let Err(x) = self
            .server
            .on_accept(ConnectionCtx {
                connection_id,
                authenticator: FramedAuthenticator::new(&mut transport),
                local_data: &mut local_data,
            })
            .await
        {
            error!("[Conn {connection_id}] Accepting connection failed: {x}");
            return;
        }

        let local_data = Arc::new(local_data);

        loop {
            let ready = transport
                .ready(Interest::READABLE | Interest::WRITABLE)
                .await
                .expect("[Conn {connection_id}] Failed to examine ready state");

            if ready.is_readable() {
                match transport.try_read_frame() {
                    Ok(Some(frame)) => match UntypedRequest::from_slice(frame.as_item()) {
                        Ok(request) => match request.to_typed_request() {
                            Ok(request) => {
                                let reply = ServerReply {
                                    origin_id: request.id.clone(),
                                    tx: tx.clone(),
                                };

                                let ctx = ServerCtx {
                                    connection_id,
                                    request,
                                    reply: reply.clone(),
                                    local_data: Arc::clone(&local_data),
                                };

                                self.server.on_request(ctx).await;
                            }
                            Err(x) => {
                                if log::log_enabled!(Level::Trace) {
                                    trace!(
                                        "[Conn {connection_id}] Failed receiving {}",
                                        String::from_utf8_lossy(&request.payload),
                                    );
                                }

                                error!("[Conn {connection_id}] Invalid request: {x}");
                            }
                        },
                        Err(x) => {
                            error!("[Conn {connection_id}] Invalid request: {x}");
                        }
                    },
                    Ok(None) => {
                        debug!("[Conn {connection_id}] Connection closed");

                        // Remove the connection from our state if it has closed
                        if let Some(state) = Weak::upgrade(&self.state) {
                            state.connections.write().await.remove(&self.id);

                            // If we have no more connections, start the timer
                            if let Some(timer) = Weak::upgrade(&self.shutdown_timer) {
                                if state.connections.read().await.is_empty() {
                                    timer.lock().await.start();
                                }
                            }
                        }
                        break;
                    }
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(x) => {
                        // NOTE: We do NOT break out of the loop, as this could happen
                        //       if someone sends bad data at any point, but does not
                        //       mean that the reader itself has failed. This can
                        //       happen from getting non-compliant typed data
                        error!("[Conn {connection_id}] {x}");
                    }
                }
            }

            // If our socket is ready to be written to, we try to get the next item from
            // the queue and process it
            if ready.is_writable() {
                match rx.try_recv() {
                    Ok(response) => {
                        // Log our message as a string, which can be expensive
                        if log_enabled!(Level::Trace) {
                            trace!(
                                "[Conn {connection_id}] Sending {}",
                                &response
                                    .to_vec()
                                    .map(|x| String::from_utf8_lossy(&x).to_string())
                                    .unwrap_or_else(|_| "<Cannot serialize>".to_string())
                            );
                        }

                        match response.to_vec() {
                            Ok(data) => match transport.try_write_frame(data) {
                                Ok(()) => continue,
                                Err(x) if x.kind() == io::ErrorKind::WouldBlock => continue,
                                Err(x) => error!("[Conn {connection_id}] Send failed: {x}"),
                            },
                            Err(x) => {
                                error!(
                                    "[Conn {connection_id}] Unable to serialize outgoing response: {x}"
                                );
                                continue;
                            }
                        }
                    }

                    // If we don't have data, we skip
                    Err(_) => continue,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InmemoryTransport, MpscListener, Request, ServerConfig};
    use async_trait::async_trait;
    use std::time::Duration;

    pub struct TestServer(ServerConfig);

    #[async_trait]
    impl Server for TestServer {
        type Request = u16;
        type Response = String;
        type LocalData = ();

        fn config(&self) -> ServerConfig {
            self.0.clone()
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

        let _server = ServerExt::start(TestServer(ServerConfig::default()), listener)
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
            TestServer(ServerConfig {
                shutdown: Shutdown::Lonely(Duration::from_millis(100)),
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
            TestServer(ServerConfig {
                shutdown: Shutdown::Lonely(Duration::from_millis(100)),
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
            TestServer(ServerConfig {
                shutdown: Shutdown::Lonely(Duration::from_millis(100)),
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
            TestServer(ServerConfig {
                shutdown: Shutdown::After(Duration::from_millis(100)),
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
            TestServer(ServerConfig {
                shutdown: Shutdown::After(Duration::from_millis(100)),
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
            TestServer(ServerConfig {
                shutdown: Shutdown::Never,
            }),
            listener,
        )
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(!server.is_finished(), "Server shutdown when it should not!");
    }
}
