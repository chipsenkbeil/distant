use crate::{
    utils::Timer, GenericServerRef, Listener, Request, Response, Server, ServerConnection,
    ServerCtx, ServerRef, ServerReply, ServerState, TypedAsyncRead, TypedAsyncWrite,
};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io,
    sync::{Arc, Weak},
};
use tokio::sync::{mpsc, Mutex};

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
    fn start<L, R, W>(self, listener: L) -> io::Result<Box<dyn ServerRef>>
    where
        L: Listener<Output = (W, R)> + 'static,
        R: TypedAsyncRead<Request<Self::Request>> + Send + 'static,
        W: TypedAsyncWrite<Response<Self::Response>> + Send + 'static;
}

impl<S, Req, Res, Data> ServerExt for S
where
    S: Server<Request = Req, Response = Res, LocalData = Data> + Sync + 'static,
    Req: DeserializeOwned + Send + Sync + 'static,
    Res: Serialize + Send + 'static,
    Data: Default + Send + Sync + 'static,
{
    type Request = Req;
    type Response = Res;

    fn start<L, R, W>(self, listener: L) -> io::Result<Box<dyn ServerRef>>
    where
        L: Listener<Output = (W, R)> + 'static,
        R: TypedAsyncRead<Request<Self::Request>> + Send + 'static,
        W: TypedAsyncWrite<Response<Self::Response>> + Send + 'static,
    {
        let server = Arc::new(self);
        let state = Arc::new(ServerState::new());

        let task = tokio::spawn(task(server, Arc::clone(&state), listener));

        Ok(Box::new(GenericServerRef { state, task }))
    }
}

async fn task<S, Req, Res, Data, L, R, W>(server: Arc<S>, state: Arc<ServerState>, mut listener: L)
where
    S: Server<Request = Req, Response = Res, LocalData = Data> + Sync + 'static,
    Req: DeserializeOwned + Send + Sync + 'static,
    Res: Serialize + Send + 'static,
    Data: Default + Send + Sync + 'static,
    L: Listener<Output = (W, R)> + 'static,
    R: TypedAsyncRead<Request<Req>> + Send + 'static,
    W: TypedAsyncWrite<Response<Res>> + Send + 'static,
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
    let mut shutdown_timer = match config.shutdown_after {
        Some(duration) => Some(Timer::new(duration, async move {
            let _ = shutdown_tx.send(()).await;
        })),
        None => None,
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
        let (mut writer, mut reader) = tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok(x) => x,
                    Err(x) => {
                        error!("Server no longer accepting connections: {}", x);
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
                    config.shutdown_after.unwrap_or_default().as_secs_f32(),
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

        // Create some default data for the new connection and pass it
        // to the callback prior to processing new requests
        let local_data = {
            let mut data = Data::default();
            server.on_accept(&mut data).await;
            Arc::new(data)
        };

        // Start a writer task that reads from a channel and forwards all
        // data through the writer
        let (tx, mut rx) = mpsc::channel::<Response<Res>>(1);
        connection.writer_task = Some(tokio::spawn(async move {
            while let Some(data) = rx.recv().await {
                // trace!("[Conn {}] Sending {:?}", connection_id, data.payload);
                if let Err(x) = writer.write(data).await {
                    error!("[Conn {}] Failed to send {:?}", connection_id, x);
                    break;
                }
            }
        }));

        // Start a reader task that reads requests and processes them
        // using the provided handler
        let weak_state = Arc::downgrade(&state);
        let weak_shutdown_timer = shutdown_timer
            .as_ref()
            .map(Arc::downgrade)
            .unwrap_or_default();
        connection.reader_task = Some(tokio::spawn(async move {
            loop {
                match reader.read().await {
                    Ok(Some(request)) => {
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

                        server.on_request(ctx).await;
                    }
                    Ok(None) => {
                        debug!("[Conn {}] Connection closed", connection_id);

                        // Remove the connection from our state if it has closed
                        if let Some(state) = Weak::upgrade(&weak_state) {
                            state.connections.write().await.remove(&connection_id);

                            // If we have no more connections, start the timer
                            if let Some(timer) = Weak::upgrade(&weak_shutdown_timer) {
                                if state.connections.read().await.is_empty() {
                                    timer.lock().await.start();
                                }
                            }
                        }
                        break;
                    }
                    Err(x) => {
                        // NOTE: We do NOT break out of the loop, as this could happen
                        //       if someone sends bad data at any point, but does not
                        //       mean that the reader itself has failed. This can
                        //       happen from getting non-compliant typed data
                        error!("[Conn {}] {}", connection_id, x);
                    }
                }
            }
        }));

        state
            .connections
            .write()
            .await
            .insert(connection_id, connection);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntoSplit, MpscListener, MpscTransport, MpscTransportReadHalf, MpscTransportWriteHalf,
        ServerConfig,
    };
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
        mpsc::Sender<(
            MpscTransportWriteHalf<Response<String>>,
            MpscTransportReadHalf<Request<u16>>,
        )>,
        MpscListener<(
            MpscTransportWriteHalf<Response<String>>,
            MpscTransportReadHalf<Request<u16>>,
        )>,
    ) {
        MpscListener::channel(buffer)
    }

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        // Create a test listener where we will forward a connection
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (mut transport, connection) =
            MpscTransport::<Request<u16>, Response<String>>::pair(100);
        tx.send(connection.into_split())
            .await
            .expect("Failed to feed listener a connection");

        let _server = ServerExt::start(TestServer(ServerConfig::default()), listener)
            .expect("Failed to start server");

        transport
            .write(Request::new(123))
            .await
            .expect("Failed to send request");

        let response: Response<String> = transport.read().await.unwrap().unwrap();
        assert_eq!(response.payload, "hello");
    }

    #[tokio::test]
    async fn should_shutdown_if_no_connections_received_after_n_secs_when_config_set() {
        let (_tx, listener) = make_listener(100);

        let server = ServerExt::start(
            TestServer(ServerConfig {
                shutdown_after: Some(Duration::from_millis(100)),
            }),
            listener,
        )
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(server.is_finished(), "Server shutdown not triggered!");
    }

    #[tokio::test]
    async fn should_shutdown_if_last_connection_terminated_and_then_no_connections_after_n_secs() {
        // Create a test listener where we will forward a connection
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (transport, connection) = MpscTransport::<Request<u16>, Response<String>>::pair(100);
        tx.send(connection.into_split())
            .await
            .expect("Failed to feed listener a connection");

        let server = ServerExt::start(
            TestServer(ServerConfig {
                shutdown_after: Some(Duration::from_millis(100)),
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
    async fn should_not_shutdown_as_long_as_a_connection_exists() {
        // Create a test listener where we will forward a connection
        let (tx, listener) = make_listener(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (_transport, connection) = MpscTransport::<Request<u16>, Response<String>>::pair(100);
        tx.send(connection.into_split())
            .await
            .expect("Failed to feed listener a connection");

        let server = ServerExt::start(
            TestServer(ServerConfig {
                shutdown_after: Some(Duration::from_millis(100)),
            }),
            listener,
        )
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(!server.is_finished(), "Server shutdown when it should not!");
    }

    #[tokio::test]
    async fn should_never_shutdown_if_config_not_set() {
        let (_tx, listener) = make_listener(100);

        let server = ServerExt::start(
            TestServer(ServerConfig {
                shutdown_after: None,
            }),
            listener,
        )
        .expect("Failed to start server");

        // Wait for some time
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(!server.is_finished(), "Server shutdown when it should not!");
    }
}
