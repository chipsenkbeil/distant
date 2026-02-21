use std::net::IpAddr;
use std::{fmt, io};

use async_trait::async_trait;
use tokio::net::{TcpStream, ToSocketAddrs};

use super::{Interest, Ready, Reconnectable, Transport};

/// Represents a [`Transport`] that leverages a TCP stream
pub struct TcpTransport {
    pub(crate) addr: IpAddr,
    pub(crate) port: u16,
    pub(crate) inner: TcpStream,
}

impl TcpTransport {
    /// Creates a new stream by connecting to a remote machine at the specified
    /// IP address and port
    pub async fn connect(addrs: impl ToSocketAddrs) -> io::Result<Self> {
        let stream = TcpStream::connect(addrs).await?;
        let addr = stream.peer_addr()?;
        Ok(Self {
            addr: addr.ip(),
            port: addr.port(),
            inner: stream,
        })
    }

    /// Returns the IP address that the stream is connected to
    pub fn ip_addr(&self) -> IpAddr {
        self.addr
    }

    /// Returns the port that the stream is connected to
    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Debug for TcpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpTransport")
            .field("addr", &self.addr)
            .field("port", &self.port)
            .finish()
    }
}

#[async_trait]
impl Reconnectable for TcpTransport {
    async fn reconnect(&mut self) -> io::Result<()> {
        self.inner = TcpStream::connect((self.addr, self.port)).await?;
        Ok(())
    }
}

#[async_trait]
impl Transport for TcpTransport {
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.try_read(buf)
    }

    fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.try_write(buf)
    }

    async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        self.inner.ready(interest).await
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv6Addr, SocketAddr};

    use test_log::test;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;

    use super::*;
    use crate::net::common::TransportExt;

    async fn find_ephemeral_addr() -> SocketAddr {
        // Start a listener on a distinct port, get its port, and kill it
        // NOTE: This is a race condition as something else could bind to
        //       this port inbetween us killing it and us attempting to
        //       connect to it. We're willing to take that chance
        let addr = IpAddr::V6(Ipv6Addr::LOCALHOST);

        let listener = TcpListener::bind((addr, 0))
            .await
            .expect("Failed to bind on an ephemeral port");

        let port = listener
            .local_addr()
            .expect("Failed to look up ephemeral port")
            .port();

        SocketAddr::from((addr, port))
    }

    async fn start_and_run_server(tx: oneshot::Sender<SocketAddr>) -> io::Result<()> {
        let addr = find_ephemeral_addr().await;

        // Start listening at the distinct address
        let listener = TcpListener::bind(addr).await?;

        // Send the address back to our main test thread
        tx.send(addr).map_err(|x| io::Error::other(x.to_string()))?;

        run_server(listener).await
    }

    async fn run_server(listener: TcpListener) -> io::Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // Get the connection
        let (mut conn, _) = listener.accept().await?;

        // Send some data to the connection (10 bytes)
        conn.write_all(b"hello conn").await?;

        // Receive some data from the connection (12 bytes)
        let mut buf: [u8; 12] = [0; 12];
        let _ = conn.read_exact(&mut buf).await?;
        assert_eq!(&buf, b"hello server");

        Ok(())
    }

    #[test(tokio::test)]
    async fn should_fail_to_connect_if_nothing_listening() {
        let addr = find_ephemeral_addr().await;

        // Now this should fail as we've stopped what was listening
        TcpTransport::connect(addr).await.expect_err(&format!(
            "Unexpectedly succeeded in connecting to ghost address: {}",
            addr
        ));
    }

    #[test(tokio::test)]
    async fn should_be_able_to_read_and_write_data() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for a connection, send data,
        // and receive data that it will return in the task
        let task: JoinHandle<io::Result<()>> = tokio::spawn(start_and_run_server(tx));

        // Wait for the server to be ready
        let addr = rx.await.expect("Failed to get server server address");

        // Connect to the socket, send some bytes, and get some bytes
        let mut buf: [u8; 10] = [0; 10];

        let conn = TcpTransport::connect(&addr)
            .await
            .expect("Conn failed to connect");

        // Continually read until we get all of the data
        conn.read_exact(&mut buf)
            .await
            .expect("Conn failed to read");
        assert_eq!(&buf, b"hello conn");

        conn.write_all(b"hello server")
            .await
            .expect("Conn failed to write");

        // Verify that the task has completed by waiting on it
        let _ = task.await.expect("Server task failed unexpectedly");
    }

    #[test(tokio::test)]
    async fn should_be_able_to_reconnect() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for a connection, send data,
        // and receive data that it will return in the task
        let task: JoinHandle<io::Result<()>> = tokio::spawn(start_and_run_server(tx));

        // Wait for the server to be ready
        let addr = rx.await.expect("Failed to get server server address");

        // Connect to the server
        let mut conn = TcpTransport::connect(&addr)
            .await
            .expect("Conn failed to connect");

        // Kill the server to make the connection fail
        task.abort();

        // Verify the connection fails by trying to read from it (should get connection reset)
        conn.readable()
            .await
            .expect("Failed to wait for conn to be readable");
        let res = conn.read_exact(&mut [0; 10]).await;
        assert!(
            matches!(res, Ok(0) | Err(_)),
            "Unexpected read result: {res:?}"
        );

        // Restart the server
        let task: JoinHandle<io::Result<()>> = tokio::spawn(run_server(
            TcpListener::bind(addr)
                .await
                .expect("Failed to rebind server"),
        ));

        // Reconnect to the socket, send some bytes, and get some bytes
        let mut buf: [u8; 10] = [0; 10];
        conn.reconnect().await.expect("Conn failed to reconnect");

        // Continually read until we get all of the data
        conn.read_exact(&mut buf)
            .await
            .expect("Conn failed to read");
        assert_eq!(&buf, b"hello conn");

        conn.write_all(b"hello server")
            .await
            .expect("Conn failed to write");

        // Verify that the task has completed by waiting on it
        let _ = task.await.expect("Server task failed unexpectedly");
    }
}
