use crate::DataStream;
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
        ToSocketAddrs,
    },
};

/// Represents a data stream for a TCP stream
pub struct TcpStream {
    pub(crate) addr: IpAddr,
    pub(crate) port: u16,
    pub(crate) inner: tokio::net::TcpStream,
}

impl TcpStream {
    /// Creates a new stream by connecting to a remote machine at the specified
    /// IP address and port
    pub async fn connect(addrs: impl ToSocketAddrs) -> io::Result<Self> {
        let stream = tokio::net::TcpStream::connect(addrs).await?;
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

impl fmt::Debug for TcpStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TcpStream")
            .field("addr", &self.addr)
            .field("port", &self.port)
            .finish()
    }
}

impl DataStream for TcpStream {
    type Read = OwnedReadHalf;
    type Write = OwnedWriteHalf;

    fn into_split(self) -> (Self::Read, Self::Write) {
        tokio::net::TcpStream::into_split(self.inner)
    }
}

impl AsyncRead for TcpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for TcpStream {
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
