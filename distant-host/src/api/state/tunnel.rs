use std::collections::HashMap;
use std::io;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use distant_core::net::server::Reply;
use distant_core::protocol::{Response, TunnelDirection, TunnelId, TunnelInfo};
use log::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// Holds information related to active TCP tunnels on the server.
pub struct TunnelState {
    channel: TunnelChannel,
    task: JoinHandle<()>,
}

impl Drop for TunnelState {
    /// Aborts the task that handles tunnel operations and management.
    fn drop(&mut self) {
        self.abort();
    }
}

impl TunnelState {
    /// Creates a new tunnel state, spawning the background actor task.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(tunnel_task(tx.clone(), rx));

        Self {
            channel: TunnelChannel { tx },
            task,
        }
    }

    /// Aborts the tunnel task.
    pub fn abort(&self) {
        self.task.abort();
    }
}

impl Deref for TunnelState {
    type Target = TunnelChannel;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

/// Channel to communicate with the tunnel actor task.
#[derive(Clone)]
pub struct TunnelChannel {
    tx: mpsc::Sender<InnerTunnelMsg>,
}

impl Default for TunnelChannel {
    /// Creates a new channel that is closed by default.
    fn default() -> Self {
        let (tx, _) = mpsc::channel(1);
        Self { tx }
    }
}

impl TunnelChannel {
    /// Opens a forward tunnel by connecting to the specified host and port.
    ///
    /// Data received from the remote TCP connection is streamed back via the reply channel
    /// as `TunnelData` responses.
    pub async fn open(
        &self,
        host: String,
        port: u16,
        reply: Box<dyn Reply<Data = Response>>,
    ) -> io::Result<TunnelId> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerTunnelMsg::Open {
                host,
                port,
                reply,
                cb,
            })
            .await
            .map_err(|_| io::Error::other("Internal tunnel task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to tunnel open dropped"))?
    }

    /// Starts a reverse tunnel listener on the specified host and port.
    ///
    /// Returns the listener's tunnel id and the actual bound port. Incoming connections
    /// are reported via `TunnelIncoming` responses, and their data is streamed via
    /// `TunnelData` responses through the reply channel.
    pub async fn listen(
        &self,
        host: String,
        port: u16,
        reply: Box<dyn Reply<Data = Response>>,
    ) -> io::Result<(TunnelId, u16)> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerTunnelMsg::Listen {
                host,
                port,
                reply,
                cb,
            })
            .await
            .map_err(|_| io::Error::other("Internal tunnel task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to tunnel listen dropped"))?
    }

    /// Writes data to an active tunnel connection.
    pub async fn write(&self, id: TunnelId, data: Vec<u8>) -> io::Result<()> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerTunnelMsg::Write { id, data, cb })
            .await
            .map_err(|_| io::Error::other("Internal tunnel task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to tunnel write dropped"))?
    }

    /// Closes an active tunnel or listener. If the target is a listener, all of its
    /// accepted sub-tunnels are also closed.
    pub async fn close(&self, id: TunnelId) -> io::Result<()> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerTunnelMsg::Close { id, cb })
            .await
            .map_err(|_| io::Error::other("Internal tunnel task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to tunnel close dropped"))?
    }

    /// Lists all active tunnels and listeners.
    pub async fn list(&self) -> io::Result<Vec<TunnelInfo>> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerTunnelMsg::List { cb })
            .await
            .map_err(|_| io::Error::other("Internal tunnel task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to tunnel list dropped"))?
    }
}

/// Internal message to pass to our task below to perform some action.
enum InnerTunnelMsg {
    Open {
        host: String,
        port: u16,
        reply: Box<dyn Reply<Data = Response>>,
        cb: oneshot::Sender<io::Result<TunnelId>>,
    },
    Listen {
        host: String,
        port: u16,
        reply: Box<dyn Reply<Data = Response>>,
        cb: oneshot::Sender<io::Result<(TunnelId, u16)>>,
    },
    Write {
        id: TunnelId,
        data: Vec<u8>,
        cb: oneshot::Sender<io::Result<()>>,
    },
    Close {
        id: TunnelId,
        cb: oneshot::Sender<io::Result<()>>,
    },
    List {
        cb: oneshot::Sender<io::Result<Vec<TunnelInfo>>>,
    },
    /// Sent by a connection's reader task when the TCP connection closes.
    InternalRemove { id: TunnelId },
    /// Sent by a listener's accept loop to register a new sub-tunnel.
    InternalRegisterSubTunnel {
        listener_id: TunnelId,
        tunnel_id: TunnelId,
        host: String,
        port: u16,
        write_tx: mpsc::Sender<Vec<u8>>,
        task: JoinHandle<()>,
    },
}

/// An entry in the tunnel map.
enum TunnelEntry {
    /// A direct forward tunnel or an incoming sub-tunnel from a listener.
    Connection {
        info: TunnelInfo,
        write_tx: mpsc::Sender<Vec<u8>>,
        task: JoinHandle<()>,
    },
    /// A reverse tunnel listener that accepts incoming connections.
    Listener {
        info: TunnelInfo,
        sub_tunnel_ids: Vec<TunnelId>,
        task: JoinHandle<()>,
    },
}

async fn tunnel_task(tx: mpsc::Sender<InnerTunnelMsg>, mut rx: mpsc::Receiver<InnerTunnelMsg>) {
    let next_id = Arc::new(AtomicU32::new(1));
    let mut tunnels: HashMap<TunnelId, TunnelEntry> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        match msg {
            InnerTunnelMsg::Open {
                host,
                port,
                reply,
                cb,
            } => {
                let stream = match TcpStream::connect((&*host, port)).await {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = cb.send(Err(e));
                        continue;
                    }
                };

                let id = next_id.fetch_add(1, Ordering::Relaxed);
                let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(1024);
                let tx_clone = tx.clone();

                let task = tokio::spawn(connection_task(id, stream, reply, write_rx, tx_clone));

                tunnels.insert(
                    id,
                    TunnelEntry::Connection {
                        info: TunnelInfo {
                            id,
                            direction: TunnelDirection::Forward,
                            host,
                            port,
                        },
                        write_tx,
                        task,
                    },
                );

                let _ = cb.send(Ok(id));
            }

            InnerTunnelMsg::Listen {
                host,
                port,
                reply,
                cb,
            } => {
                let listener = match TcpListener::bind((&*host, port)).await {
                    Ok(l) => l,
                    Err(e) => {
                        let _ = cb.send(Err(e));
                        continue;
                    }
                };

                let actual_port = match listener.local_addr() {
                    Ok(addr) => addr.port(),
                    Err(e) => {
                        let _ = cb.send(Err(e));
                        continue;
                    }
                };

                let listener_id = next_id.fetch_add(1, Ordering::Relaxed);
                let tx_clone = tx.clone();
                let next_id_clone = Arc::clone(&next_id);

                let task = tokio::spawn(listener_task(
                    listener_id,
                    listener,
                    reply,
                    tx_clone,
                    next_id_clone,
                ));

                tunnels.insert(
                    listener_id,
                    TunnelEntry::Listener {
                        info: TunnelInfo {
                            id: listener_id,
                            direction: TunnelDirection::Reverse,
                            host,
                            port: actual_port,
                        },
                        sub_tunnel_ids: Vec::new(),
                        task,
                    },
                );

                let _ = cb.send(Ok((listener_id, actual_port)));
            }

            InnerTunnelMsg::Write { id, data, cb } => {
                let _ = cb.send(match tunnels.get(&id) {
                    Some(TunnelEntry::Connection { write_tx, .. }) => {
                        write_tx.try_send(data).map_err(|_| {
                            io::Error::other(format!("Tunnel {id} write channel full or closed"))
                        })
                    }
                    Some(TunnelEntry::Listener { .. }) => {
                        Err(io::Error::other(format!("Cannot write to listener {id}")))
                    }
                    None => Err(io::Error::other(format!("No tunnel found with id {id}"))),
                });
            }

            InnerTunnelMsg::Close { id, cb } => {
                let _ = cb.send(close_tunnel(id, &mut tunnels));
            }

            InnerTunnelMsg::List { cb } => {
                let list: Vec<TunnelInfo> = tunnels
                    .values()
                    .map(|entry| match entry {
                        TunnelEntry::Connection { info, .. } => info.clone(),
                        TunnelEntry::Listener { info, .. } => info.clone(),
                    })
                    .collect();
                let _ = cb.send(Ok(list));
            }

            InnerTunnelMsg::InternalRemove { id } => {
                // Only remove connections, not listeners. If a sub-tunnel closes,
                // also remove its id from the parent listener's sub_tunnel_ids.
                if let Some(TunnelEntry::Connection { .. }) = tunnels.get(&id) {
                    tunnels.remove(&id);

                    // Clean up the sub-tunnel reference from any parent listener
                    for entry in tunnels.values_mut() {
                        if let TunnelEntry::Listener { sub_tunnel_ids, .. } = entry {
                            sub_tunnel_ids.retain(|&sub_id| sub_id != id);
                        }
                    }
                }
            }

            InnerTunnelMsg::InternalRegisterSubTunnel {
                listener_id,
                tunnel_id,
                host,
                port,
                write_tx,
                task,
            } => {
                // Register the sub-tunnel as a Connection entry
                tunnels.insert(
                    tunnel_id,
                    TunnelEntry::Connection {
                        info: TunnelInfo {
                            id: tunnel_id,
                            direction: TunnelDirection::Reverse,
                            host,
                            port,
                        },
                        write_tx,
                        task,
                    },
                );

                // Add to parent listener's sub_tunnel_ids
                if let Some(TunnelEntry::Listener { sub_tunnel_ids, .. }) =
                    tunnels.get_mut(&listener_id)
                {
                    sub_tunnel_ids.push(tunnel_id);
                }
            }
        }
    }
}

/// Closes a tunnel or listener, aborting its tasks and removing it from the map.
/// If the target is a listener, all sub-tunnels are also closed.
fn close_tunnel(id: TunnelId, tunnels: &mut HashMap<TunnelId, TunnelEntry>) -> io::Result<()> {
    match tunnels.remove(&id) {
        Some(TunnelEntry::Connection { task, .. }) => {
            task.abort();
            Ok(())
        }
        Some(TunnelEntry::Listener {
            sub_tunnel_ids,
            task,
            ..
        }) => {
            task.abort();
            for sub_id in sub_tunnel_ids {
                if let Some(TunnelEntry::Connection { task, .. }) = tunnels.remove(&sub_id) {
                    task.abort();
                }
            }
            Ok(())
        }
        None => Err(io::Error::other(format!("No tunnel found with id {id}"))),
    }
}

/// Manages the I/O for a single TCP connection (forward or sub-tunnel).
///
/// Reads from the TCP stream and sends `TunnelData` responses via the reply channel.
/// Writes data received on `write_rx` to the TCP stream. Sends `TunnelClosed` and
/// an `InternalRemove` message when the connection ends.
async fn connection_task(
    id: TunnelId,
    stream: TcpStream,
    reply: Box<dyn Reply<Data = Response>>,
    mut write_rx: mpsc::Receiver<Vec<u8>>,
    tx: mpsc::Sender<InnerTunnelMsg>,
) {
    let (mut read_half, mut write_half) = stream.into_split();

    // Spawn writer sub-task
    let write_task = tokio::spawn(async move {
        while let Some(data) = write_rx.recv().await {
            if write_half.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    // Read loop
    let mut buf = vec![0u8; 8192];
    loop {
        match read_half.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let data = buf[..n].to_vec();
                if reply.send(Response::TunnelData { id, data }).is_err() {
                    break;
                }
            }
            Err(e) => {
                debug!("[Tunnel {id}] Read error: {e}");
                break;
            }
        }
    }

    let _ = reply.send(Response::TunnelClosed { id });
    write_task.abort();
    let _ = tx.send(InnerTunnelMsg::InternalRemove { id }).await;
}

/// Accepts incoming connections on a `TcpListener` and spawns sub-tunnel tasks for each.
///
/// Each accepted connection is registered in the tunnel map via `InternalRegisterSubTunnel`.
/// Sends `TunnelIncoming` notifications through the reply channel.
async fn listener_task(
    listener_id: TunnelId,
    listener: TcpListener,
    reply: Box<dyn Reply<Data = Response>>,
    tx: mpsc::Sender<InnerTunnelMsg>,
    next_id: Arc<AtomicU32>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, peer_addr)) => {
                let tunnel_id = next_id.fetch_add(1, Ordering::Relaxed);
                let peer_str = Some(peer_addr.to_string());
                let host = peer_addr.ip().to_string();
                let port = peer_addr.port();

                if reply
                    .send(Response::TunnelIncoming {
                        listener_id,
                        tunnel_id,
                        peer_addr: peer_str,
                    })
                    .is_err()
                {
                    break;
                }

                let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(1024);
                let sub_reply = reply.clone_reply();
                let tx_clone = tx.clone();

                let task = tokio::spawn(connection_task(
                    tunnel_id, stream, sub_reply, write_rx, tx_clone,
                ));

                // Register the sub-tunnel in the main actor's map
                let _ = tx
                    .send(InnerTunnelMsg::InternalRegisterSubTunnel {
                        listener_id,
                        tunnel_id,
                        host,
                        port,
                        write_tx,
                        task,
                    })
                    .await;
            }
            Err(e) => {
                debug!("[Tunnel {listener_id}] Accept error: {e}");
                break;
            }
        }
    }

    let _ = reply.send(Response::TunnelClosed { id: listener_id });
    let _ = tx
        .send(InnerTunnelMsg::InternalRemove { id: listener_id })
        .await;
}
