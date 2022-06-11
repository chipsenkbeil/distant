use crate::{Listener, PortRange, TcpTransport};
use async_trait::async_trait;
use std::{fmt, io, net::IpAddr};
use tokio::net::TcpListener as TokioTcpListener;

/// Represents a listener for incoming connections over TCP
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

#[async_trait]
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
