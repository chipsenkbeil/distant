use crate::{IntoSplit, RawTransport, RawTransportRead, RawTransportWrite};
use std::{
    fmt, io,
    net::IpAddr,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream, ToSocketAddrs,
    },
};

/// Represents a [`RawTransport`] that leverages a TCP stream
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

impl RawTransport for TcpTransport {}
impl RawTransportRead for TcpTransport {}
impl RawTransportWrite for TcpTransport {}

impl RawTransportRead for OwnedReadHalf {}
impl RawTransportWrite for OwnedWriteHalf {}

impl IntoSplit for TcpTransport {
    type Read = OwnedReadHalf;
    type Write = OwnedWriteHalf;

    fn into_split(self) -> (Self::Write, Self::Read) {
        let (r, w) = TcpStream::into_split(self.inner);
        (w, r)
    }
}

impl AsyncRead for TcpTransport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for TcpTransport {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv6Addr, SocketAddr};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
        task::JoinHandle,
    };

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

    #[tokio::test]
    async fn should_fail_to_connect_if_nothing_listening() {
        let addr = find_ephemeral_addr().await;

        // Now this should fail as we've stopped what was listening
        TcpTransport::connect(addr).await.expect_err(&format!(
            "Unexpectedly succeeded in connecting to ghost address: {}",
            addr
        ));
    }

    #[tokio::test]
    async fn should_be_able_to_send_and_receive_data() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for a connection, send data,
        // and receive data that it will return in the task
        let task: JoinHandle<io::Result<()>> = tokio::spawn(async move {
            let addr = find_ephemeral_addr().await;

            // Start listening at the distinct address
            let listener = TcpListener::bind(addr).await?;

            // Send the address back to our main test thread
            tx.send(addr)
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x.to_string()))?;

            // Get the connection
            let (mut conn, _) = listener.accept().await?;

            // Send some data to the connection (10 bytes)
            conn.write_all(b"hello conn").await?;

            // Receive some data from the connection (12 bytes)
            let mut buf: [u8; 12] = [0; 12];
            let _ = conn.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"hello server");

            Ok(())
        });

        // Wait for the server to be ready
        let addr = rx.await.expect("Failed to get server server address");

        // Connect to the socket, send some bytes, and get some bytes
        let mut buf: [u8; 10] = [0; 10];

        let mut conn = TcpTransport::connect(&addr)
            .await
            .expect("Conn failed to connect");
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
