use std::io;

use anyhow::Context;
use distant_core::constants::TUNNEL_RELAY_BUFFER_SIZE;
use distant_core::protocol::TunnelDirection;
use distant_core::{Channel, ChannelExt, RemoteTunnelListener};
use log::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::CliResult;

/// Parses a tunnel spec in the format `PORT:HOST:PORT`.
///
/// Uses `rfind(':')` to handle IPv6 hosts like `[::1]`.
fn parse_tunnel_spec(spec: &str) -> io::Result<(u16, String, u16)> {
    let first_colon = spec
        .find(':')
        .ok_or_else(|| io::Error::other(format!("Invalid tunnel spec: {spec}")))?;
    let local_port: u16 = spec[..first_colon]
        .parse()
        .map_err(|e| io::Error::other(format!("Invalid local port: {e}")))?;

    let rest = &spec[first_colon + 1..];
    let last_colon = rest
        .rfind(':')
        .ok_or_else(|| io::Error::other(format!("Invalid tunnel spec: {spec}")))?;
    let host = rest[..last_colon].to_string();
    let remote_port: u16 = rest[last_colon + 1..]
        .parse()
        .map_err(|e| io::Error::other(format!("Invalid remote port: {e}")))?;

    Ok((local_port, host, remote_port))
}

/// Handles `distant tunnel open` — local port forwarding.
///
/// Binds a local TCP listener on `localhost:LOCAL_PORT` and for each accepted
/// connection, opens a remote tunnel to `REMOTE_HOST:REMOTE_PORT` and relays
/// data bidirectionally until Ctrl+C.
pub async fn handle_open(mut channel: Channel, spec: &str) -> CliResult {
    let (local_port, remote_host, remote_port) =
        parse_tunnel_spec(spec).context("Failed to parse tunnel spec")?;

    let listener = TcpListener::bind(format!("127.0.0.1:{local_port}"))
        .await
        .with_context(|| format!("Failed to bind local listener on port {local_port}"))?;

    let actual_port = listener
        .local_addr()
        .context("Failed to get local address")?
        .port();

    println!("Forwarding 127.0.0.1:{actual_port} -> {remote_host}:{remote_port}");
    println!("Press Ctrl+C to stop");

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (tcp_stream, peer_addr) = result
                    .context("Failed to accept local connection")?;
                debug!("Accepted local connection from {peer_addr}");

                let host = remote_host.clone();
                let mut tunnel = channel
                    .tunnel_open(host.clone(), remote_port)
                    .await
                    .with_context(|| {
                        format!("Failed to open tunnel to {host}:{remote_port}")
                    })?;

                debug!("Tunnel {} opened to {host}:{remote_port}", tunnel.id());

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
pub async fn handle_listen(mut channel: Channel, spec: &str) -> CliResult {
    let (remote_port, local_host, local_port) =
        parse_tunnel_spec(spec).context("Failed to parse tunnel spec")?;

    let mut listener =
        RemoteTunnelListener::listen(channel.clone(), local_host.clone(), remote_port)
            .await
            .with_context(|| {
                format!("Failed to start remote listener on {local_host}:{remote_port}")
            })?;

    println!(
        "Listening on remote port {} -> {local_host}:{local_port}",
        listener.port()
    );
    println!("Press Ctrl+C to stop");

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
                    .tunnel_open(local_host.clone(), local_port)
                    .await
                    .with_context(|| {
                        format!("Failed to open tunnel for incoming connection {}", incoming.tunnel_id)
                    })?;

                let host = local_host.clone();
                let port = local_port;

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
                let _ = listener.close().await;
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
    fn parse_tunnel_spec_simple() {
        let (local, host, remote) = parse_tunnel_spec("8080:db-host:5432").unwrap();
        assert_eq!(local, 8080);
        assert_eq!(host, "db-host");
        assert_eq!(remote, 5432);
    }

    #[test]
    fn parse_tunnel_spec_localhost() {
        let (local, host, remote) = parse_tunnel_spec("3000:localhost:3000").unwrap();
        assert_eq!(local, 3000);
        assert_eq!(host, "localhost");
        assert_eq!(remote, 3000);
    }

    #[test]
    fn parse_tunnel_spec_ipv6() {
        let (local, host, remote) = parse_tunnel_spec("8080:[::1]:5432").unwrap();
        assert_eq!(local, 8080);
        assert_eq!(host, "[::1]");
        assert_eq!(remote, 5432);
    }

    #[test]
    fn parse_tunnel_spec_missing_colon() {
        let result = parse_tunnel_spec("8080");
        assert!(result.is_err());
    }

    #[test]
    fn parse_tunnel_spec_invalid_local_port() {
        let result = parse_tunnel_spec("abc:host:5432");
        assert!(result.is_err());
    }

    #[test]
    fn parse_tunnel_spec_invalid_remote_port() {
        let result = parse_tunnel_spec("8080:host:abc");
        assert!(result.is_err());
    }

    #[test]
    fn parse_tunnel_spec_only_one_colon() {
        let result = parse_tunnel_spec("8080:host");
        assert!(result.is_err());
    }
}
