mod handler;
mod state;

use state::State;

use crate::{
    data::{Request, Response},
    net::{DataStream, Listener, SecretKey, Transport, TransportReadHalf, TransportWriteHalf},
    server::{
        utils::{ConnTracker, ShutdownTask},
        PortRange,
    },
};
use log::*;
use std::{net::IpAddr, sync::Arc};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    net::TcpListener,
    sync::{mpsc, Mutex},
    task::{JoinError, JoinHandle},
    time::Duration,
};

/// Represents a server that listens for requests, processes them, and sends responses
pub struct DistantServer {
    port: u16,
    auth_key: Arc<SecretKey>,
    conn_task: JoinHandle<()>,
}

impl DistantServer {
    /// Bind to an IP address and port from the given range, taking an optional shutdown duration
    /// that will shutdown the server if there is no active connection after duration
    pub async fn bind(
        addr: IpAddr,
        port: PortRange,
        shutdown_after: Option<Duration>,
        max_msg_capacity: usize,
    ) -> io::Result<Self> {
        debug!("Binding to {} in range {}", addr, port);
        let listener = TcpListener::bind(port.make_socket_addrs(addr).as_slice()).await?;

        let port = listener.local_addr()?.port();
        debug!("Bound to port: {}", port);

        Ok(Self::initialize(
            listener,
            port,
            shutdown_after,
            max_msg_capacity,
        ))
    }

    /// Initialize a distant server using the provided listener
    pub fn initialize<T, L>(
        listener: L,
        port: u16,
        shutdown_after: Option<Duration>,
        max_msg_capacity: usize,
    ) -> Self
    where
        T: DataStream + Send + 'static,
        L: Listener<Conn = T> + 'static,
    {
        // Build our state for the server
        let state: Arc<Mutex<State>> = Arc::new(Mutex::new(State::default()));
        let auth_key = Arc::new(SecretKey::default());
        let (shutdown, tracker) = ShutdownTask::maybe_initialize(shutdown_after);

        // Spawn our connection task
        let auth_key_2 = Arc::clone(&auth_key);
        let conn_task = tokio::spawn(async move {
            connection_loop(
                listener,
                state,
                auth_key_2,
                tracker,
                shutdown,
                max_msg_capacity,
            )
            .await
        });

        Self {
            port,
            auth_key,
            conn_task,
        }
    }

    /// Returns the port this server is bound to
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Returns a string representing the auth key as hex
    pub fn to_unprotected_hex_auth_key(&self) -> String {
        hex::encode(self.auth_key.unprotected_as_bytes())
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

async fn connection_loop<T, L>(
    listener: L,
    state: Arc<Mutex<State>>,
    auth_key: Arc<SecretKey>,
    tracker: Option<Arc<Mutex<ConnTracker>>>,
    shutdown: Option<ShutdownTask>,
    max_msg_capacity: usize,
) where
    T: DataStream,
    L: Listener<Conn = T>,
{
    let inner = async move {
        loop {
            match listener.accept().await {
                Ok(conn) => {
                    let conn_id = rand::random();
                    debug!(
                        "<Conn @ {}> Established against {}",
                        conn_id,
                        conn.to_connection_tag()
                    );
                    if let Err(x) = on_new_conn(
                        conn,
                        conn_id,
                        Arc::clone(&state),
                        Arc::clone(&auth_key),
                        tracker.as_ref().map(Arc::clone),
                        max_msg_capacity,
                    )
                    .await
                    {
                        error!("<Conn @ {}> Failed handshake: {}", conn_id, x);
                    }
                }
                Err(x) => {
                    error!("Listener failed: {}", x);
                    break;
                }
            }
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
async fn on_new_conn<T>(
    conn: T,
    conn_id: usize,
    state: Arc<Mutex<State>>,
    auth_key: Arc<SecretKey>,
    tracker: Option<Arc<Mutex<ConnTracker>>>,
    max_msg_capacity: usize,
) -> io::Result<JoinHandle<()>>
where
    T: DataStream,
{
    // Update our tracker to reflect the new connection
    if let Some(ct) = tracker.as_ref() {
        ct.lock().await.increment();
    }

    // Establish a proper connection via a handshake,
    // discarding the connection otherwise
    let transport = Transport::from_handshake(conn, Some(auth_key)).await?;

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
async fn request_loop<T>(
    conn_id: usize,
    state: Arc<Mutex<State>>,
    mut transport: TransportReadHalf<T>,
    tx: mpsc::Sender<Response>,
) where
    T: AsyncRead + Send + Unpin + 'static,
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
async fn response_loop<T>(
    conn_id: usize,
    mut transport: TransportWriteHalf<T>,
    mut rx: mpsc::Receiver<Response>,
) where
    T: AsyncWrite + Send + Unpin + 'static,
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

    #[test]
    fn wait_should_return_ok_when_all_inner_tasks_complete() {
        todo!();
    }

    #[test]
    fn wait_should_return_error_when_server_aborted() {
        todo!();
    }

    #[test]
    fn abort_should_abort_inner_tasks_and_all_connections() {
        todo!();
    }

    #[test]
    fn server_should_shutdown_if_no_connections_after_shutdown_duration() {
        todo!();
    }

    #[test]
    fn server_shutdown_should_abort_all_connections() {
        todo!();
    }

    #[test]
    fn server_should_execute_requests_and_return_responses() {
        todo!();
    }
}
