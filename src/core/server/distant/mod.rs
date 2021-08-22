mod handler;
mod port;
mod state;
mod utils;

pub use port::{PortRange, PortRangeParseError};
use state::State;

use crate::core::{
    data::{Request, Response},
    net::{SecretKey, Transport, TransportReadHalf, TransportWriteHalf},
};
use log::*;
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio::{
    io,
    net::{tcp, TcpListener, TcpStream},
    runtime::Handle,
    sync::{mpsc, Mutex, Notify},
    task::{JoinError, JoinHandle},
    time::Duration,
};

/// Represents a server that listens for requests, processes them, and sends responses
pub struct Server {
    port: u16,
    state: Arc<Mutex<State<SocketAddr>>>,
    auth_key: Arc<SecretKey>,
    notify: Arc<Notify>,
    conn_task: JoinHandle<()>,
}

impl Server {
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
        let state: Arc<Mutex<State<SocketAddr>>> = Arc::new(Mutex::new(State::default()));
        let auth_key = Arc::new(SecretKey::default());
        let (ct, notify) = utils::new_shutdown_task(Handle::current(), shutdown_after);

        // Spawn our connection task
        let state_2 = Arc::clone(&state);
        let auth_key_2 = Arc::clone(&auth_key);
        let notify_2 = Arc::clone(&notify);
        let conn_task = tokio::spawn(async move {
            connection_loop(
                listener,
                state_2,
                auth_key_2,
                ct,
                notify_2,
                max_msg_capacity,
            )
            .await
        });

        Ok(Self {
            port,
            state,
            auth_key,
            notify,
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

    /// Shutdown the server
    pub fn shutdown(&self) {
        self.notify.notify_one()
    }
}

async fn connection_loop(
    listener: TcpListener,
    state: Arc<Mutex<State<SocketAddr>>>,
    auth_key: Arc<SecretKey>,
    tracker: Arc<Mutex<utils::ConnTracker>>,
    notify: Arc<Notify>,
    max_msg_capacity: usize,
) {
    loop {
        tokio::select! {
            result = listener.accept() => {match result {
                Ok((conn, addr)) => {
                    if let Err(x) = on_new_conn(
                        conn,
                        addr,
                        Arc::clone(&state),
                        Arc::clone(&auth_key),
                        Arc::clone(&tracker),
                        max_msg_capacity
                    ).await {
                        error!("<Conn @ {}> Failed handshake: {}", addr, x);
                    }
                }
                Err(x) => {
                    error!("Listener failed: {}", x);
                    break;
                }
            }}
            _ = notify.notified() => {
                warn!("Reached shutdown timeout, so terminating");
                break;
            }
        }
    }
}

/// Processes a new connection, performing a handshake, and then spawning two tasks to handle
/// input and output, returning join handles for the input and output tasks respectively
async fn on_new_conn(
    conn: TcpStream,
    addr: SocketAddr,
    state: Arc<Mutex<State<SocketAddr>>>,
    auth_key: Arc<SecretKey>,
    tracker: Arc<Mutex<utils::ConnTracker>>,
    max_msg_capacity: usize,
) -> io::Result<(JoinHandle<()>, JoinHandle<()>)> {
    // Establish a proper connection via a handshake,
    // discarding the connection otherwise
    let transport = Transport::from_handshake(conn, Some(auth_key)).await?;

    // Split the transport into read and write halves so we can handle input
    // and output concurrently
    let (t_read, t_write) = transport.into_split();
    let (tx, rx) = mpsc::channel(max_msg_capacity);
    let ct_2 = Arc::clone(&tracker);

    // Spawn a new task that loops to handle requests from the client
    let req_task = tokio::spawn({
        let f = request_loop(addr, Arc::clone(&state), t_read, tx);

        let state = Arc::clone(&state);
        async move {
            ct_2.lock().await.increment();
            f.await;
            state.lock().await.cleanup_client(addr).await;
            ct_2.lock().await.decrement();
        }
    });

    // Spawn a new task that loops to handle responses to the client
    let res_task = tokio::spawn(async move { response_loop(addr, t_write, rx).await });

    Ok((req_task, res_task))
}

/// Repeatedly reads in new requests, processes them, and sends their responses to the
/// response loop
async fn request_loop(
    addr: SocketAddr,
    state: Arc<Mutex<State<SocketAddr>>>,
    mut transport: TransportReadHalf<tcp::OwnedReadHalf>,
    tx: mpsc::Sender<Response>,
) {
    loop {
        match transport.receive::<Request>().await {
            Ok(Some(req)) => {
                debug!(
                    "<Conn @ {}> Received request of type{} {}",
                    addr,
                    if req.payload.len() > 1 { "s" } else { "" },
                    req.to_payload_type_string()
                );

                if let Err(x) = handler::process(addr, Arc::clone(&state), req, tx.clone()).await {
                    error!("<Conn @ {}> {}", addr, x);
                    break;
                }
            }
            Ok(None) => {
                info!("<Conn @ {}> Closed connection", addr);
                break;
            }
            Err(x) => {
                error!("<Conn @ {}> {}", addr, x);
                break;
            }
        }
    }
}

/// Repeatedly sends responses out over the wire
async fn response_loop(
    addr: SocketAddr,
    mut transport: TransportWriteHalf<tcp::OwnedWriteHalf>,
    mut rx: mpsc::Receiver<Response>,
) {
    while let Some(res) = rx.recv().await {
        if let Err(x) = transport.send(res).await {
            error!("<Conn @ {}> {}", addr, x);
            break;
        }
    }
}
