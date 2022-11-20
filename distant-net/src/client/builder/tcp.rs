use super::Connector;
use crate::common::TcpTransport;
use async_trait::async_trait;
use std::io;
use tokio::net::ToSocketAddrs;

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

#[async_trait]
impl<T: ToSocketAddrs + Send> Connector for TcpConnector<T> {
    type Transport = TcpTransport;

    async fn connect(self) -> io::Result<Self::Transport> {
        TcpTransport::connect(self.addr).await
    }
}
