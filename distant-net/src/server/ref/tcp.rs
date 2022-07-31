use crate::{ServerRef, ServerState};
use std::net::IpAddr;

/// Reference to a TCP server instance
pub struct TcpServerRef {
    pub(crate) addr: IpAddr,
    pub(crate) port: u16,
    pub(crate) inner: Box<dyn ServerRef>,
}

impl TcpServerRef {
    pub fn new(addr: IpAddr, port: u16, inner: Box<dyn ServerRef>) -> Self {
        Self { addr, port, inner }
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

impl ServerRef for TcpServerRef {
    fn state(&self) -> &ServerState {
        self.inner.state()
    }

    fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    fn abort(&self) {
        self.inner.abort();
    }
}
