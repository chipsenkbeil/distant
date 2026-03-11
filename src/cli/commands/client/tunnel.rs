use std::io;
use std::str::FromStr;

use anyhow::Context;
use distant_core::constants::TUNNEL_RELAY_BUFFER_SIZE;
use distant_core::protocol::TunnelDirection;
use distant_core::{Channel, ChannelExt, RemoteTunnelListener};
use log::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::CliResult;

/// Parsed tunnel spec from the `PORT:HOST:PORT` format.
///
/// Field semantics depend on the command:
/// - `tunnel open`: `bind_port` is local, `host`:`peer_port` is remote target
/// - `tunnel listen`: `bind_port` is remote, `host`:`peer_port` is local target
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelSpec {
    /// The port on the side that binds/listens (left side of spec).
    pub bind_port: u16,
    /// The host to connect to (middle of spec).
    pub host: String,
    /// The port to connect to on the host (right side of spec).
    pub peer_port: u16,
}

impl FromStr for TunnelSpec {
    type Err = io::Error;

    /// Parses a tunnel spec in the format `PORT:HOST:PORT`.
    ///
    /// Uses `rfind(':')` to handle IPv6 hosts like `[::1]`.
    fn from_str(spec: &str) -> Result<Self, Self::Err> {
        let first_colon = spec
            .find(':')
            .ok_or_else(|| io::Error::other(format!("Invalid tunnel spec: {spec}")))?;
        let bind_port: u16 = spec[..first_colon]
            .parse()
            .map_err(|e| io::Error::other(format!("Invalid bind port: {e}")))?;

        let rest = &spec[first_colon + 1..];
        let last_colon = rest
            .rfind(':')
            .ok_or_else(|| io::Error::other(format!("Invalid tunnel spec: {spec}")))?;
        let host = rest[..last_colon].to_string();
        let peer_port: u16 = rest[last_colon + 1..]
            .parse()
            .map_err(|e| io::Error::other(format!("Invalid peer port: {e}")))?;

        Ok(Self {
            bind_port,
            host,
            peer_port,
        })
    }
}

/// Handles `distant tunnel open` — local port forwarding.
///
/// Binds a local TCP listener on `localhost:LOCAL_PORT` and for each accepted
/// connection, opens a remote tunnel to `REMOTE_HOST:REMOTE_PORT` and relays
/// data bidirectionally until Ctrl+C.
pub async fn handle_open(channel: Channel, spec: &str, foreground: bool) -> CliResult {
    let spec: TunnelSpec = spec.parse().context("Failed to parse tunnel spec")?;

    let listener = TcpListener::bind(format!("127.0.0.1:{}", spec.bind_port))
        .await
        .with_context(|| format!("Failed to bind local listener on port {}", spec.bind_port))?;

    let actual_port = listener
        .local_addr()
        .context("Failed to get local address")?
        .port();

    println!(
        "Forwarding 127.0.0.1:{actual_port} -> {}:{}",
        spec.host, spec.peer_port
    );

    if foreground {
        println!("Press Ctrl+C to stop");
        run_open_loop(channel, listener, spec).await
    } else {
        println!("Use 'distant tunnel close <id>' to stop");
        tokio::spawn(async move {
            if let Err(e) = run_open_loop(channel, listener, spec).await {
                debug!("Tunnel accept loop error: {e}");
            }
        });
        Ok(())
    }
}

/// Accept loop for forward tunnel: accepts local connections and relays them.
async fn run_open_loop(mut channel: Channel, listener: TcpListener, spec: TunnelSpec) -> CliResult {
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (tcp_stream, peer_addr) = result
                    .context("Failed to accept local connection")?;
                debug!("Accepted local connection from {peer_addr}");

                let host = spec.host.clone();
                let port = spec.peer_port;
                let mut tunnel = channel
                    .tunnel_open(host.clone(), port)
                    .await
                    .with_context(|| {
                        format!("Failed to open tunnel to {host}:{port}")
                    })?;

                debug!("Tunnel {} opened to {host}:{port}", tunnel.id());

                // Take writer and reader from the tunnel
                let writer = tunnel.writer.take()
                    .ok_or_else(|| anyhow::anyhow!("Tunnel writer already taken"))?;
                let reader = tunnel.reader.take()
                    .ok_or_else(|| anyhow::anyhow!("Tunnel reader already taken"))?;

                // Spawn a relay task for this connection
                tokio::spawn(async move {
                    if let Err(e) = relay_local_to_tunnel(tcp_stream, writer, reader).await {
                        debug!("Relay finished: {e}");
                    }
                    // Close the tunnel when the relay finishes
                    let _ = tunnel.close().await;
                });
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down tunnel");
                break;
            }
        }
    }

    Ok(())
}

/// Handles `distant tunnel listen` — reverse port forwarding.
///
/// Opens a remote listener on `REMOTE_PORT` and for each incoming connection,
/// connects locally to `LOCAL_HOST:LOCAL_PORT` and relays data bidirectionally
/// until Ctrl+C.
pub async fn handle_listen(channel: Channel, spec: &str, foreground: bool) -> CliResult {
    let spec: TunnelSpec = spec.parse().context("Failed to parse tunnel spec")?;

    let mut listener =
        RemoteTunnelListener::listen(channel.clone(), spec.host.clone(), spec.bind_port)
            .await
            .with_context(|| {
                format!(
                    "Failed to start remote listener on {}:{}",
                    spec.host, spec.bind_port
                )
            })?;

    println!(
        "Listening on remote port {} -> {}:{}",
        listener.port(),
        spec.host,
        spec.peer_port
    );

    if foreground {
        println!("Press Ctrl+C to stop");
        run_listen_loop(channel, &mut listener, spec).await?;
        let _ = listener.close().await;
        Ok(())
    } else {
        println!("Use 'distant tunnel close <id>' to stop");
        tokio::spawn(async move {
            if let Err(e) = run_listen_loop(channel, &mut listener, spec).await {
                debug!("Tunnel listen loop error: {e}");
            }
            let _ = listener.close().await;
        });
        Ok(())
    }
}

/// Accept loop for reverse tunnel: accepts incoming connections and relays them locally.
async fn run_listen_loop(
    mut channel: Channel,
    listener: &mut RemoteTunnelListener,
    spec: TunnelSpec,
) -> CliResult {
    loop {
        tokio::select! {
            incoming = listener.next() => {
                let incoming = match incoming {
                    Some(incoming) => incoming,
                    None => {
                        debug!("Remote listener closed");
                        break;
                    }
                };

                debug!(
                    "Incoming connection on tunnel {} (peer: {:?})",
                    incoming.tunnel_id,
                    incoming.peer_addr,
                );

                // Open a tunnel for this incoming connection using the tunnel_id
                // The server has already created a sub-tunnel; we need to open it
                let mut tunnel = channel
                    .tunnel_open(spec.host.clone(), spec.peer_port)
                    .await
                    .with_context(|| {
                        format!("Failed to open tunnel for incoming connection {}", incoming.tunnel_id)
                    })?;

                let host = spec.host.clone();
                let port = spec.peer_port;

                // Take writer and reader from the tunnel
                let writer = tunnel.writer.take()
                    .ok_or_else(|| anyhow::anyhow!("Tunnel writer already taken"))?;
                let reader = tunnel.reader.take()
                    .ok_or_else(|| anyhow::anyhow!("Tunnel reader already taken"))?;

                tokio::spawn(async move {
                    // Connect to local target
                    let tcp_stream = match tokio::net::TcpStream::connect(format!("{host}:{port}")).await {
                        Ok(s) => s,
                        Err(e) => {
                            debug!("Failed to connect to local {host}:{port}: {e}");
                            let _ = tunnel.close().await;
                            return;
                        }
                    };

                    if let Err(e) = relay_local_to_tunnel(tcp_stream, writer, reader).await {
                        debug!("Relay finished: {e}");
                    }
                    let _ = tunnel.close().await;
                });
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nShutting down listener");
                break;
            }
        }
    }

    Ok(())
}

/// Handles `distant tunnel close` — closes an active tunnel by ID.
pub async fn handle_close(mut channel: Channel, id: u32) -> CliResult {
    channel
        .tunnel_close(id)
        .await
        .with_context(|| format!("Failed to close tunnel {id}"))?;

    println!("Tunnel {id} closed");
    Ok(())
}

/// Handles `distant tunnel list` — lists all active tunnels and listeners.
pub async fn handle_list(mut channel: Channel) -> CliResult {
    let info = channel.status().await.context("Failed to get status")?;
    let entries = info.tunnels;

    if entries.is_empty() {
        println!("No active tunnels");
    } else {
        println!(
            "{:<6} {:<10} {:<30} {:<6}",
            "ID", "Direction", "Host", "Port"
        );
        for entry in entries {
            let direction = match entry.direction {
                TunnelDirection::Forward => "forward",
                TunnelDirection::Reverse => "reverse",
            };
            println!(
                "{:<6} {:<10} {:<30} {:<6}",
                entry.id, direction, entry.host, entry.port
            );
        }
    }

    Ok(())
}

/// Relays data bidirectionally between a local TCP stream and a remote tunnel.
///
/// Splits the TCP stream into read/write halves, then runs two tasks:
/// - local read -> tunnel write
/// - tunnel read -> local write
///
/// Returns when either direction encounters an error or EOF.
async fn relay_local_to_tunnel(
    tcp_stream: tokio::net::TcpStream,
    mut writer: distant_core::RemoteTunnelWriter,
    mut reader: distant_core::RemoteTunnelReader,
) -> io::Result<()> {
    let (mut tcp_read, mut tcp_write) = tcp_stream.into_split();

    let local_to_remote = tokio::spawn(async move {
        let mut buf = vec![0u8; TUNNEL_RELAY_BUFFER_SIZE];
        loop {
            let n = tcp_read.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            writer.write(buf[..n].to_vec()).await?;
        }
        io::Result::Ok(())
    });

    let remote_to_local = tokio::spawn(async move {
        loop {
            let data = reader.read().await?;
            if data.is_empty() {
                break;
            }
            tcp_write.write_all(&data).await?;
        }
        io::Result::Ok(())
    });

    // Wait for either direction to finish
    tokio::select! {
        result = local_to_remote => {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => debug!("Local-to-remote relay error: {e}"),
                Err(e) => debug!("Local-to-remote task panicked: {e}"),
            }
        }
        result = remote_to_local => {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => debug!("Remote-to-local relay error: {e}"),
                Err(e) => debug!("Remote-to-local task panicked: {e}"),
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_spec_simple() {
        let spec: TunnelSpec = "8080:db-host:5432".parse().unwrap();
        assert_eq!(spec.bind_port, 8080);
        assert_eq!(spec.host, "db-host");
        assert_eq!(spec.peer_port, 5432);
    }

    #[test]
    fn tunnel_spec_localhost() {
        let spec: TunnelSpec = "3000:localhost:3000".parse().unwrap();
        assert_eq!(spec.bind_port, 3000);
        assert_eq!(spec.host, "localhost");
        assert_eq!(spec.peer_port, 3000);
    }

    #[test]
    fn tunnel_spec_ipv6() {
        let spec: TunnelSpec = "8080:[::1]:5432".parse().unwrap();
        assert_eq!(spec.bind_port, 8080);
        assert_eq!(spec.host, "[::1]");
        assert_eq!(spec.peer_port, 5432);
    }

    #[test]
    fn tunnel_spec_missing_colon() {
        assert!("8080".parse::<TunnelSpec>().is_err());
    }

    #[test]
    fn tunnel_spec_invalid_bind_port() {
        assert!("abc:host:5432".parse::<TunnelSpec>().is_err());
    }

    #[test]
    fn tunnel_spec_invalid_peer_port() {
        assert!("8080:host:abc".parse::<TunnelSpec>().is_err());
    }

    #[test]
    fn tunnel_spec_only_one_colon() {
        assert!("8080:host".parse::<TunnelSpec>().is_err());
    }
}
