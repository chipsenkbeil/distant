use crate::{
    data::{Request, Response},
    net::{Transport, TransportReadHalf, TransportWriteHalf},
    opt::{CommonOpt, ConvertToIpAddrError, ListenSubcommand},
};
use derive_more::{Display, Error, From};
use fork::{daemon, Fork};
use log::*;
use orion::aead::SecretKey;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::{
    io,
    net::TcpListener,
    sync::{mpsc, oneshot, Mutex},
};

mod handler;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    ConvertToIpAddrError(ConvertToIpAddrError),
    ForkError,
    IoError(io::Error),
}

/// Holds state relevant to the server
#[derive(Default)]
struct State {
    /// Map of all processes running on the server
    processes: HashMap<usize, Process>,

    /// List of processes that will be killed when a client drops
    client_processes: HashMap<SocketAddr, Vec<usize>>,
}

impl State {
    /// Cleans up state associated with a particular client
    pub async fn cleanup_client(&mut self, addr: SocketAddr) {
        if let Some(ids) = self.client_processes.remove(&addr) {
            for id in ids {
                if let Some(process) = self.processes.remove(&id) {
                    if let Err(_) = process.kill_tx.send(()) {
                        error!(
                            "Client {} failed to send process {} kill signal",
                            id, process.id
                        );
                    }
                }
            }
        }
    }
}

/// Represents an actively-running process maintained by the server
struct Process {
    pub id: usize,
    pub cmd: String,
    pub args: Vec<String>,
    pub stdin_tx: mpsc::Sender<Vec<u8>>,
    pub kill_tx: oneshot::Sender<()>,
}

pub fn run(cmd: ListenSubcommand, opt: CommonOpt) -> Result<(), Error> {
    if cmd.daemon {
        // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
        match daemon(false, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async { run_async(cmd, opt, true).await })?;
            }
            Ok(Fork::Parent(pid)) => {
                info!("[distant detached, pid = {}]", pid);
                if let Err(_) = fork::close_fd() {
                    return Err(Error::ForkError);
                }
            }
            Err(_) => return Err(Error::ForkError),
        }
    } else {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async { run_async(cmd, opt, false).await })?;
    }

    Ok(())
}

async fn run_async(cmd: ListenSubcommand, _opt: CommonOpt, is_forked: bool) -> Result<(), Error> {
    let addr = cmd.host.to_ip_addr(cmd.use_ipv6)?;
    let socket_addrs = cmd.port.make_socket_addrs(addr);

    // If specified, change the current working directory of this program
    if let Some(path) = cmd.current_dir.as_ref() {
        debug!("Setting current directory to {:?}", path);
        std::env::set_current_dir(path)?;
    }

    debug!("Binding to {} in range {}", addr, cmd.port);
    let listener = TcpListener::bind(socket_addrs.as_slice()).await?;

    let port = listener.local_addr()?.port();
    debug!("Bound to port: {}", port);

    let key = Arc::new(SecretKey::default());

    // Print information about port, key, etc.
    publish_data(port, &key);

    // For the child, we want to fully disconnect it from pipes, which we do now
    if is_forked {
        if let Err(_) = fork::close_fd() {
            return Err(Error::ForkError);
        }
    }

    // Build our state for the server
    let state = Arc::new(Mutex::new(State::default()));

    // Wait for a client connection, then spawn a new task to handle
    // receiving data from the client
    while let Ok((client, _)) = listener.accept().await {
        // Grab the client's remote address for later logging purposes
        let addr = match client.peer_addr() {
            Ok(addr) => {
                info!("<Client @ {}> Established connection", addr);
                addr
            }
            Err(x) => {
                error!("Unable to examine client's peer address: {}", x);
                continue;
            }
        };

        // Establish a proper connection via a handshake, discarding the connection otherwise
        let transport = match Transport::from_handshake(client, Arc::clone(&key)).await {
            Ok(transport) => transport,
            Err(x) => {
                error!("<Client @ {}> Failed handshake: {}", addr, x);
                continue;
            }
        };

        // Split the transport into read and write halves so we can handle input
        // and output concurrently
        let (t_read, t_write) = transport.into_split();
        let (tx, rx) = mpsc::channel(cmd.max_msg_capacity as usize);

        // Spawn a new task that loops to handle requests from the client
        tokio::spawn({
            let f = request_loop(addr, Arc::clone(&state), t_read, tx);

            let state = Arc::clone(&state);
            async move {
                f.await;
                state.lock().await.cleanup_client(addr).await;
            }
        });

        // Spawn a new task that loops to handle responses to the client
        tokio::spawn(async move { response_loop(addr, t_write, rx).await });
    }

    Ok(())
}

/// Repeatedly reads in new requests, processes them, and sends their responses to the
/// response loop
async fn request_loop(
    addr: SocketAddr,
    state: Arc<Mutex<State>>,
    mut transport: TransportReadHalf,
    tx: mpsc::Sender<Response>,
) {
    loop {
        match transport.receive::<Request>().await {
            Ok(Some(req)) => {
                trace!(
                    "<Client @ {}> Received request of type {}",
                    addr,
                    req.payload.as_ref()
                );

                if let Err(x) = handler::process(addr, Arc::clone(&state), req, tx.clone()).await {
                    error!("<Client @ {}> {}", addr, x);
                    break;
                }
            }
            Ok(None) => {
                info!("<Client @ {}> Closed connection", addr);
                break;
            }
            Err(x) => {
                error!("<Client @ {}> {}", addr, x);
                break;
            }
        }
    }
}

/// Repeatedly sends responses out over the wire
async fn response_loop(
    addr: SocketAddr,
    mut transport: TransportWriteHalf,
    mut rx: mpsc::Receiver<Response>,
) {
    while let Some(res) = rx.recv().await {
        if let Err(x) = transport.send(res).await {
            error!("<Client @ {}> {}", addr, x);
            break;
        }
    }
}

/// Prints out the port and **secret auth key** to share with a client when
/// establishing communication. This is **highly unsafe** and should only be
/// done when the server is launched over a secure channel such as SSH.
fn publish_data(port: u16, key: &SecretKey) {
    println!(
        "DISTANT DATA {} {}",
        port,
        hex::encode(key.unprotected_as_bytes())
    );
}
