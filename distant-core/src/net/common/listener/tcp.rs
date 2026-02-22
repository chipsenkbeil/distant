use std::net::IpAddr;
use std::{fmt, io};

use tokio::net::TcpListener as TokioTcpListener;

use super::Listener;
use crate::net::common::{PortRange, TcpTransport};

/// Represents a [`Listener`] for incoming connections over TCP
pub struct TcpListener {
    addr: IpAddr,
    port: u16,
    inner: TokioTcpListener,
}

impl TcpListener {
    /// Creates a new listener by binding to the specified IP address and port
    /// in the given port range
    pub async fn bind(addr: IpAddr, port: impl Into<PortRange>) -> io::Result<Self> {
        let listener =
            TokioTcpListener::bind(port.into().make_socket_addrs(addr).as_slice()).await?;

        // Get the port that we bound to
        let port = listener.local_addr()?.port();

        Ok(Self {
            addr,
            port,
            inner: listener,
        })
    }

    /// Returns the IP address that the listener is bound to
    pub fn ip_addr(&self) -> IpAddr {
        self.addr
    }

    /// Returns the port that the listener is bound to
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Debug for TcpListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpListener")
            .field("addr", &self.addr)
            .field("port", &self.port)
            .finish()
    }
}

impl Listener for TcpListener {
    type Output = TcpTransport;

    async fn accept(&mut self) -> io::Result<Self::Output> {
        let (stream, peer_addr) = TokioTcpListener::accept(&self.inner).await?;
        Ok(TcpTransport {
            addr: peer_addr.ip(),
            port: peer_addr.port(),
            inner: stream,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv6Addr, SocketAddr};

    use test_log::test;
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;

    use super::*;
    use crate::net::common::TransportExt;

    #[test(tokio::test)]
    async fn should_fail_to_bind_if_port_already_bound() {
        let addr = IpAddr::V6(Ipv6Addr::LOCALHOST);
        let port = 0; // Ephemeral port

        // Listen at some port
        let listener = TcpListener::bind(addr, port)
            .await
            .expect("Unexpectedly failed to bind first time");

        // Get the actual port we bound to
        let port = listener.port();

        // Now this should fail as we're already bound to the address and port
        TcpListener::bind(addr, port).await.expect_err(&format!(
            "Unexpectedly succeeded in binding a second time to {}:{}",
            addr, port,
        ));
    }

    #[test(tokio::test)]
    async fn should_be_able_to_receive_connections_and_read_and_write_data_with_them() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for two connections and then
        // return the success or failure
        let task: JoinHandle<io::Result<()>> = tokio::spawn(async move {
            let addr = IpAddr::V6(Ipv6Addr::LOCALHOST);
            let port = 0; // Ephemeral port

            // Listen at the address and port
            let mut listener = TcpListener::bind(addr, port).await?;

            // Send the name back to our main test thread
            tx.send(SocketAddr::from((addr, listener.port())))
                .map_err(|x| io::Error::other(x.to_string()))?;

            // Get first connection
            let conn_1 = listener.accept().await?;

            // Send some data to the first connection (12 bytes)
            conn_1.write_all(b"hello conn 1").await?;

            // Get some data from the first connection (14 bytes)
            let mut buf: [u8; 14] = [0; 14];
            let _ = conn_1.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"hello server 1");

            // Get second connection
            let conn_2 = listener.accept().await?;

            // Send some data on to second connection (12 bytes)
            conn_2.write_all(b"hello conn 2").await?;

            // Get some data from the second connection (14 bytes)
            let mut buf: [u8; 14] = [0; 14];
            let _ = conn_2.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"hello server 2");

            Ok(())
        });

        // Wait for the server to be ready
        let address = rx.await.expect("Failed to get server address");

        // Connect to the listener twice, sending some bytes and receiving some bytes from each
        let mut buf: [u8; 12] = [0; 12];

        let conn = TcpTransport::connect(&address)
            .await
            .expect("Conn 1 failed to connect");
        conn.write_all(b"hello server 1")
            .await
            .expect("Conn 1 failed to write");
        conn.read_exact(&mut buf)
            .await
            .expect("Conn 1 failed to read");
        assert_eq!(&buf, b"hello conn 1");

        let conn = TcpTransport::connect(&address)
            .await
            .expect("Conn 2 failed to connect");
        conn.write_all(b"hello server 2")
            .await
            .expect("Conn 2 failed to write");
        conn.read_exact(&mut buf)
            .await
            .expect("Conn 2 failed to read");
        assert_eq!(&buf, b"hello conn 2");

        // Verify that the task has completed by waiting on it
        let _ = task.await.expect("Listener task failed unexpectedly");
    }
}
