use std::io;
use std::sync::atomic::{AtomicU32, Ordering};

use log::*;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::client::{ChannelExt, RemoteTunnelListener, relay_tcp_to_tunnel};
use crate::net::common::ConnectionId;
use crate::net::manager::data::{ManagedTunnelId, ManagedTunnelInfo};
use crate::protocol::TunnelDirection;

use super::InternalRawChannel;
use super::connection::ManagerChannel;

static NEXT_MANAGED_TUNNEL_ID: AtomicU32 = AtomicU32::new(1);

/// A tunnel whose lifecycle is managed by the manager process.
///
/// The tunnel's relay loop runs in a spawned task. Dropping the `ManagedTunnel`
/// does **not** abort the task — call [`abort`](Self::abort) explicitly.
pub struct ManagedTunnel {
    pub id: ManagedTunnelId,
    pub connection_id: ConnectionId,
    pub info: ManagedTunnelInfo,
    task: JoinHandle<()>,
    manager_channel: ManagerChannel,
}

impl ManagedTunnel {
    /// Aborts the relay task and closes the underlying manager channel.
    pub fn abort(&self) {
        self.task.abort();
        let _ = self.manager_channel.close();
    }
}

/// Starts a forward tunnel (local TCP listener → remote target) inside the
/// manager process.
///
/// The caller should open the [`InternalRawChannel`] while briefly holding the
/// connection lock, then pass it here for the async setup.
///
/// Returns the managed tunnel and the actual bound local port (which may differ
/// from `bind_port` when `0` is passed).
///
/// # Errors
///
/// Returns an error if binding the local TCP listener fails.
pub async fn start_forward_tunnel(
    internal: InternalRawChannel,
    connection_id: ConnectionId,
    bind_port: u16,
    remote_host: String,
    remote_port: u16,
) -> io::Result<(ManagedTunnel, u16)> {
    let (mut channel, manager_channel) = internal.into_parts();

    let listener = TcpListener::bind(format!("127.0.0.1:{bind_port}"))
        .await
        .map_err(|e| io::Error::other(format!("Failed to bind on port {bind_port}: {e}")))?;
    let actual_port = listener.local_addr()?.port();

    let id = NEXT_MANAGED_TUNNEL_ID.fetch_add(1, Ordering::Relaxed);
    let host = remote_host.clone();
    let port = remote_port;

    let task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((tcp_stream, peer_addr)) => {
                    debug!("[ManagedTunnel {id}] Accepted connection from {peer_addr}");
                    let host = host.clone();

                    let mut tunnel = match channel.tunnel_open(host.clone(), port).await {
                        Ok(t) => t,
                        Err(e) => {
                            debug!(
                                "[ManagedTunnel {id}] Failed to open tunnel to {host}:{port}: {e}"
                            );
                            continue;
                        }
                    };

                    let writer = match tunnel.writer.take() {
                        Some(w) => w,
                        None => continue,
                    };
                    let reader = match tunnel.reader.take() {
                        Some(r) => r,
                        None => continue,
                    };

                    tokio::spawn(async move {
                        if let Err(e) = relay_tcp_to_tunnel(tcp_stream, writer, reader).await {
                            debug!("Forward relay finished: {e}");
                        }
                        let _ = tunnel.close().await;
                    });
                }
                Err(e) => {
                    debug!("[ManagedTunnel {id}] Accept error: {e}");
                    break;
                }
            }
        }
    });

    let info = ManagedTunnelInfo {
        id,
        connection_id,
        direction: TunnelDirection::Forward,
        bind_port: actual_port,
        remote_host,
        remote_port,
    };

    Ok((
        ManagedTunnel {
            id,
            connection_id,
            info,
            task,
            manager_channel,
        },
        actual_port,
    ))
}

/// Starts a reverse tunnel (remote listener → local TCP target) inside the
/// manager process.
///
/// The caller should open the [`InternalRawChannel`] while briefly holding the
/// connection lock, then pass it here for the async setup.
///
/// Returns the managed tunnel and the actual remote port (which may differ
/// from `remote_port` when `0` is passed).
///
/// # Errors
///
/// Returns an error if the remote side fails to set up the listener.
pub async fn start_reverse_tunnel(
    internal: InternalRawChannel,
    connection_id: ConnectionId,
    remote_port: u16,
    local_host: String,
    local_port: u16,
) -> io::Result<(ManagedTunnel, u16)> {
    let (channel, manager_channel) = internal.into_parts();

    // Ask the remote to listen on the specified port
    let mut listener =
        RemoteTunnelListener::listen(channel.clone(), "0.0.0.0".to_string(), remote_port)
            .await
            .map_err(|e| {
                io::Error::other(format!(
                    "Failed to start remote listener on port {remote_port}: {e}"
                ))
            })?;

    let actual_port = listener.port();
    let id = NEXT_MANAGED_TUNNEL_ID.fetch_add(1, Ordering::Relaxed);
    let host = local_host.clone();
    let port = local_port;

    let task = tokio::spawn(async move {
        let mut channel = channel;
        loop {
            let incoming = match listener.next().await {
                Some(incoming) => incoming,
                None => {
                    debug!("[ManagedTunnel {id}] Remote listener closed");
                    break;
                }
            };

            debug!(
                "[ManagedTunnel {id}] Incoming connection (peer: {:?})",
                incoming.peer_addr,
            );

            let host = host.clone();
            let port = port;

            // Open a new forward tunnel for this incoming connection
            let mut tunnel = match channel.tunnel_open(host.clone(), port).await {
                Ok(t) => t,
                Err(e) => {
                    debug!(
                        "[ManagedTunnel {id}] Failed to open tunnel for incoming connection: {e}"
                    );
                    continue;
                }
            };

            let writer = match tunnel.writer.take() {
                Some(w) => w,
                None => continue,
            };
            let reader = match tunnel.reader.take() {
                Some(r) => r,
                None => continue,
            };

            tokio::spawn(async move {
                // Connect to local target
                let tcp_stream =
                    match tokio::net::TcpStream::connect(format!("{host}:{port}")).await {
                        Ok(s) => s,
                        Err(e) => {
                            debug!("Failed to connect to local {host}:{port}: {e}");
                            let _ = tunnel.close().await;
                            return;
                        }
                    };

                if let Err(e) = relay_tcp_to_tunnel(tcp_stream, writer, reader).await {
                    debug!("Reverse relay finished: {e}");
                }
                let _ = tunnel.close().await;
            });
        }

        let _ = listener.close().await;
    });

    let info = ManagedTunnelInfo {
        id,
        connection_id,
        direction: TunnelDirection::Reverse,
        bind_port: actual_port,
        remote_host: local_host,
        remote_port: local_port,
    };

    Ok((
        ManagedTunnel {
            id,
            connection_id,
            info,
            task,
            manager_channel,
        },
        actual_port,
    ))
}
