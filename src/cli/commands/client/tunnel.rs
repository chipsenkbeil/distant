use std::io;
use std::str::FromStr;

use anyhow::Context;
use distant_core::net::common::ConnectionId;
use distant_core::net::manager::ManagerClient;
use distant_core::protocol::TunnelDirection;

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

/// Handles `distant tunnel open` — requests the manager to start a forward tunnel.
pub async fn handle_open(
    client: &mut ManagerClient,
    connection_id: ConnectionId,
    spec: &str,
) -> CliResult {
    let spec: TunnelSpec = spec.parse().context("Failed to parse tunnel spec")?;

    let (id, port) = client
        .forward_tunnel(
            connection_id,
            spec.bind_port,
            spec.host.clone(),
            spec.peer_port,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to start forward tunnel {}:{}:{}",
                spec.bind_port, spec.host, spec.peer_port
            )
        })?;

    println!(
        "Tunnel {id} started: 127.0.0.1:{port} -> {}:{}",
        spec.host, spec.peer_port
    );
    Ok(())
}

/// Handles `distant tunnel listen` — requests the manager to start a reverse tunnel.
pub async fn handle_listen(
    client: &mut ManagerClient,
    connection_id: ConnectionId,
    spec: &str,
) -> CliResult {
    let spec: TunnelSpec = spec.parse().context("Failed to parse tunnel spec")?;

    let (id, port) = client
        .reverse_tunnel(
            connection_id,
            spec.bind_port,
            spec.host.clone(),
            spec.peer_port,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to start reverse tunnel {}:{}:{}",
                spec.bind_port, spec.host, spec.peer_port
            )
        })?;

    println!(
        "Tunnel {id} started: remote port {port} -> {}:{}",
        spec.host, spec.peer_port
    );
    Ok(())
}

/// Handles `distant tunnel close` — closes a managed tunnel by ID.
pub async fn handle_close(client: &mut ManagerClient, id: u32) -> CliResult {
    client
        .close_managed_tunnel(id)
        .await
        .with_context(|| format!("Failed to close tunnel {id}"))?;

    println!("Tunnel {id} closed");
    Ok(())
}

/// Handles `distant tunnel list` — lists all managed tunnels.
pub async fn handle_list(client: &mut ManagerClient) -> CliResult {
    let tunnels = client
        .list_managed_tunnels()
        .await
        .context("Failed to list managed tunnels")?;

    if tunnels.is_empty() {
        println!("No active tunnels");
    } else {
        println!(
            "{:<6} {:<10} {:<12} {:<30} {:<6}",
            "ID", "Direction", "Bind Port", "Remote Host", "Remote Port"
        );
        for t in tunnels {
            let direction = match t.direction {
                TunnelDirection::Forward => "forward",
                TunnelDirection::Reverse => "reverse",
            };
            println!(
                "{:<6} {:<10} {:<12} {:<30} {:<6}",
                t.id, direction, t.bind_port, t.remote_host, t.remote_port
            );
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
