mod handler;
mod process;
mod state;

pub(crate) use process::{InputChannel, ProcessKiller, ProcessPty};
use state::State;

use crate::{
    constants::MAX_MSG_CAPACITY,
    data::{Request, Response},
    server::utils::{ConnTracker, ShutdownTask},
};
use distant_net::{
    Codec, DataStream, Listener, MappedListener, PortRange, TcpListener, Transport,
    TransportReadHalf, TransportWriteHalf,
};
use log::*;
use std::{net::IpAddr, sync::Arc};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    sync::{mpsc, Mutex},
    task::{JoinError, JoinHandle},
    time::Duration,
};

/// Represents a server that listens for requests, processes them, and sends responses
pub struct DistantServer {
    conn_task: JoinHandle<()>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistantServerOptions {
    pub shutdown_after: Option<Duration>,
    pub max_msg_capacity: usize,
}

impl Default for DistantServerOptions {
    fn default() -> Self {
        Self {
            shutdown_after: None,
            max_msg_capacity: MAX_MSG_CAPACITY,
        }
    }
}

impl DistantServer {
    /// Bind to an IP address and port from the given range, taking an optional shutdown duration
    /// that will shutdown the server if there is no active connection after duration
    pub async fn bind<U>(
        addr: IpAddr,
        port: PortRange,
        codec: U,
        opts: DistantServerOptions,
    ) -> io::Result<(Self, u16)>
    where
        U: Codec + Send + Sync + 'static,
    {
        debug!("Binding to {} in range {}", addr, port);
        let listener = TcpListener::bind(addr, port).await?;

        let port = listener.port();
        debug!("Bound to port: {}", port);

        let listener = MappedListener::new(listener, move |stream| {
            Transport::new(stream, codec.clone())
        });

        Ok((Self::initialize(Box::pin(listener), opts), port))
    }

    /// Initialize a distant server using the provided listener
    pub fn initialize<T, U, L>(listener: L, opts: DistantServerOptions) -> Self
    where
        T: DataStream + Send + 'static,
        U: Codec + Send + Sync + 'static,
        L: Listener<Output = Transport<T, U>> + Unpin + 'static,
    {
        // Build our state for the server
        let state: Arc<Mutex<State>> = Arc::new(Mutex::new(State::default()));
        let (shutdown, tracker) = ShutdownTask::maybe_initialize(opts.shutdown_after);

        // Spawn our connection task
        let conn_task = tokio::spawn(async move {
            connection_loop(listener, state, tracker, shutdown, opts.max_msg_capacity).await
        });

        Self { conn_task }
    }

    /// Waits for the server to terminate
    pub async fn wait(self) -> Result<(), JoinError> {
        self.conn_task.await
    }

    /// Aborts the server by aborting the internal task handling new connections
    pub fn abort(&self) {
        self.conn_task.abort();
    }
}

async fn connection_loop<T, U, L>(
    mut listener: L,
    state: Arc<Mutex<State>>,
    tracker: Option<Arc<Mutex<ConnTracker>>>,
    shutdown: Option<ShutdownTask>,
    max_msg_capacity: usize,
) where
    T: DataStream + Send + 'static,
    U: Codec + Send + 'static,
    L: Listener<Output = Transport<T, U>> + Unpin + 'static,
{
    let inner = async move {
        loop {
            match listener.accept().await {
                Some(transport) => {
                    let conn_id = rand::random();
                    debug!("<Conn @ {}> Established", conn_id);
                    if let Err(x) = on_new_conn(
                        transport,
                        conn_id,
                        Arc::clone(&state),
                        tracker.as_ref().map(Arc::clone),
                        max_msg_capacity,
                    )
                    .await
                    {
                        error!("<Conn @ {}> Failed handshake: {}", conn_id, x);
                    }
                }
                None => {
                    info!("Listener shutting down");
                    break;
                }
            };
        }
    };

    match shutdown {
        Some(shutdown) => tokio::select! {
            _ = inner => {}
            _ = shutdown => {
                warn!("Reached shutdown timeout, so terminating");
            }
        },
        None => inner.await,
    }
}

/// Processes a new connection, performing a handshake, and then spawning two tasks to handle
/// input and output, returning join handles for the input and output tasks respectively
async fn on_new_conn<T, U>(
    transport: Transport<T, U>,
    conn_id: usize,
    state: Arc<Mutex<State>>,
    tracker: Option<Arc<Mutex<ConnTracker>>>,
    max_msg_capacity: usize,
) -> io::Result<JoinHandle<()>>
where
    T: DataStream,
    U: Codec + Send + 'static,
{
    // Update our tracker to reflect the new connection
    if let Some(ct) = tracker.as_ref() {
        ct.lock().await.increment();
    }

    // Split the transport into read and write halves so we can handle input
    // and output concurrently
    let (t_read, t_write) = transport.into_split();
    let (tx, rx) = mpsc::channel(max_msg_capacity);

    // Spawn a new task that loops to handle requests from the client
    let state_2 = Arc::clone(&state);
    let req_task = tokio::spawn(async move {
        request_loop(conn_id, state_2, t_read, tx).await;
    });

    // Spawn a new task that loops to handle responses to the client
    let res_task = tokio::spawn(async move { response_loop(conn_id, t_write, rx).await });

    // Spawn cleanup task that waits on our req & res tasks to complete
    let cleanup_task = tokio::spawn(async move {
        // Wait for both receiving and sending tasks to complete before marking
        // the connection as complete
        let _ = tokio::join!(req_task, res_task);

        state.lock().await.cleanup_connection(conn_id).await;
        if let Some(ct) = tracker.as_ref() {
            ct.lock().await.decrement();
        }
    });

    Ok(cleanup_task)
}

/// Repeatedly reads in new requests, processes them, and sends their responses to the
/// response loop
async fn request_loop<T, U>(
    conn_id: usize,
    state: Arc<Mutex<State>>,
    mut transport: TransportReadHalf<T, U>,
    tx: mpsc::Sender<Response>,
) where
    T: AsyncRead + Send + Unpin + 'static,
    U: Codec,
{
    loop {
        match transport.receive::<Request>().await {
            Ok(Some(req)) => {
                debug!(
                    "<Conn @ {}> Received request of type{} {}",
                    conn_id,
                    if req.payload.len() > 1 { "s" } else { "" },
                    req.to_payload_type_string()
                );

                if let Err(x) = handler::process(conn_id, Arc::clone(&state), req, tx.clone()).await
                {
                    error!("<Conn @ {}> {}", conn_id, x);
                    break;
                }
            }
            Ok(None) => {
                trace!("<Conn @ {}> Input from connection closed", conn_id);
                break;
            }
            Err(x) => {
                error!("<Conn @ {}> {}", conn_id, x);
                break;
            }
        }
    }

    // Properly close off any associated process' stdin given that we can't get new
    // requests to send more stdin to them
    state.lock().await.close_stdin_for_connection(conn_id);
}

/// Repeatedly sends responses out over the wire
async fn response_loop<T, U>(
    conn_id: usize,
    mut transport: TransportWriteHalf<T, U>,
    mut rx: mpsc::Receiver<Response>,
) where
    T: AsyncWrite + Send + Unpin + 'static,
    U: Codec,
{
    while let Some(res) = rx.recv().await {
        debug!(
            "<Conn @ {}> Sending response of type{} {}",
            conn_id,
            if res.payload.len() > 1 { "s" } else { "" },
            res.to_payload_type_string()
        );

        if let Err(x) = transport.send(res).await {
            error!("<Conn @ {}> {}", conn_id, x);
            break;
        }
    }

    trace!("<Conn @ {}> Output to connection closed", conn_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{RequestData, ResponseData};
    use distant_net::{InmemoryStream, PlainCodec, TestListener};

    #[allow(clippy::type_complexity)]
    fn make_transport_stream() -> (
        mpsc::Sender<Transport<InmemoryStream, PlainCodec>>,
        TestListener<Transport<InmemoryStream, PlainCodec>>,
    ) {
        TestListener::channel(1)
    }

    #[tokio::test]
    async fn wait_should_return_ok_when_all_inner_tasks_complete() {
        let (tx, stream) = make_transport_stream();

        let server = DistantServer::initialize(stream, Default::default());

        // Conclude all server tasks by closing out the listener
        drop(tx);

        let result = server.wait().await;
        assert!(result.is_ok(), "Unexpected result: {:?}", result);
    }

    #[tokio::test]
    async fn wait_should_return_error_when_server_aborted() {
        let (_tx, stream) = make_transport_stream();

        let server = DistantServer::initialize(stream, Default::default());
        server.abort();

        match server.wait().await {
            Err(x) if x.is_cancelled() => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn server_should_receive_requests_and_send_responses_to_appropriate_connections() {
        let (tx, stream) = make_transport_stream();

        let _server = DistantServer::initialize(stream, Default::default());

        // Send over a "connection"
        let (mut t1, t2) = Transport::make_pair();
        tx.send(t2).await.unwrap();

        // Send a request
        t1.send(Request::new(
            "test-tenant",
            vec![RequestData::SystemInfo {}],
        ))
        .await
        .unwrap();

        // Get a response
        let res = t1.receive::<Response>().await.unwrap().unwrap();
        assert!(res.payload.len() == 1, "Unexpected payload size");
        assert!(
            matches!(res.payload[0], ResponseData::SystemInfo { .. }),
            "Unexpected response: {:?}",
            res.payload[0]
        );
    }

    #[tokio::test]
    async fn server_should_shutdown_if_no_connections_after_shutdown_duration() {
        let (_tx, stream) = make_transport_stream();

        let server = DistantServer::initialize(
            stream,
            DistantServerOptions {
                shutdown_after: Some(Duration::from_millis(50)),
                max_msg_capacity: 1,
            },
        );

        let result = server.wait().await;
        assert!(result.is_ok(), "Unexpected result: {:?}", result);
    }
}
