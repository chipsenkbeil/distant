mod handler;
mod state;

use state::State;

use crate::core::{
    data::{Request, Response},
    net::{SecretKey, Transport, TransportReadHalf, TransportWriteHalf},
    server::{
        utils::{ConnTracker, ShutdownTask},
        PortRange,
    },
};
use futures::future::OptionFuture;
use log::*;
use std::{net::IpAddr, sync::Arc};
use tokio::{
    io,
    net::{tcp, TcpListener, TcpStream},
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

        Ok(Self {
            port,
            auth_key,
            conn_task,
        })
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
}

async fn connection_loop(
    listener: TcpListener,
    state: Arc<Mutex<State>>,
    auth_key: Arc<SecretKey>,
    tracker: Option<Arc<Mutex<ConnTracker>>>,
    shutdown: OptionFuture<ShutdownTask>,
    max_msg_capacity: usize,
) {
    let inner = async move {
        loop {
            match listener.accept().await {
                Ok((conn, addr)) => {
                    let conn_id = rand::random();
                    debug!("<Conn @ {}> Established against {}", conn_id, addr);
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
                        error!("<Conn @ {}> Failed handshake: {}", addr, x);
                    }
                }
                Err(x) => {
                    error!("Listener failed: {}", x);
                    break;
                }
            }
        }
    };

    tokio::select! {
        _ = inner => {}
        _ = shutdown => {
            warn!("Reached shutdown timeout, so terminating");
        }
    }
}

/// Processes a new connection, performing a handshake, and then spawning two tasks to handle
/// input and output, returning join handles for the input and output tasks respectively
async fn on_new_conn(
    conn: TcpStream,
    conn_id: usize,
    state: Arc<Mutex<State>>,
    auth_key: Arc<SecretKey>,
    tracker: Option<Arc<Mutex<ConnTracker>>>,
    max_msg_capacity: usize,
) -> io::Result<(JoinHandle<()>, JoinHandle<()>)> {
    // Establish a proper connection via a handshake,
    // discarding the connection otherwise
    let transport = Transport::from_handshake(conn, Some(auth_key)).await?;

    // Split the transport into read and write halves so we can handle input
    // and output concurrently
    let (t_read, t_write) = transport.into_split();
    let (tx, rx) = mpsc::channel(max_msg_capacity);

    // Spawn a new task that loops to handle requests from the client
    let req_task = tokio::spawn({
        let f = request_loop(conn_id, Arc::clone(&state), t_read, tx);

        let state = Arc::clone(&state);
        async move {
            if let Some(ct) = tracker.as_ref() {
                ct.lock().await.increment();
            }
            f.await;
            state.lock().await.cleanup_connection(conn_id).await;
            if let Some(ct) = tracker.as_ref() {
                ct.lock().await.decrement();
            }
        }
    });

    // Spawn a new task that loops to handle responses to the client
    let res_task = tokio::spawn(async move { response_loop(conn_id, t_write, rx).await });

    Ok((req_task, res_task))
}

/// Repeatedly reads in new requests, processes them, and sends their responses to the
/// response loop
async fn request_loop(
    conn_id: usize,
    state: Arc<Mutex<State>>,
    mut transport: TransportReadHalf<tcp::OwnedReadHalf>,
    tx: mpsc::Sender<Response>,
) {
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
                info!("<Conn @ {}> Closed connection", conn_id);
                break;
            }
            Err(x) => {
                error!("<Conn @ {}> {}", conn_id, x);
                break;
            }
        }
    }
}

/// Repeatedly sends responses out over the wire
async fn response_loop(
    conn_id: usize,
    mut transport: TransportWriteHalf<tcp::OwnedWriteHalf>,
    mut rx: mpsc::Receiver<Response>,
) {
    while let Some(res) = rx.recv().await {
        if let Err(x) = transport.send(res).await {
            error!("<Conn @ {}> {}", conn_id, x);
            break;
        }
    }
}
