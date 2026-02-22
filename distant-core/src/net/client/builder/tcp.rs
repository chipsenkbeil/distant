use std::io;

use tokio::net::ToSocketAddrs;

use super::Connector;
use crate::net::common::TcpTransport;

/// Implementation of [`Connector`] to support connecting via TCP.
pub struct TcpConnector<T> {
    addr: T,
}

impl<T> TcpConnector<T> {
    pub fn new(addr: T) -> Self {
        Self { addr }
    }
}

impl<T> From<T> for TcpConnector<T> {
    fn from(addr: T) -> Self {
        Self::new(addr)
    }
}

impl<T: ToSocketAddrs + Send> Connector for TcpConnector<T> {
    type Transport = TcpTransport;

    async fn connect(self) -> io::Result<Self::Transport> {
        TcpTransport::connect(self.addr).await
    }
}
