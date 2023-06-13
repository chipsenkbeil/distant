use std::future::Future;
use std::net::IpAddr;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::task::JoinError;

use super::ServerRef;

/// Reference to a TCP server instance.
pub struct TcpServerRef {
    pub(crate) addr: IpAddr,
    pub(crate) port: u16,
    pub(crate) inner: ServerRef,
}

impl TcpServerRef {
    pub fn new(addr: IpAddr, port: u16, inner: ServerRef) -> Self {
        Self { addr, port, inner }
    }

    /// Returns the IP address that the listener is bound to.
    pub fn ip_addr(&self) -> IpAddr {
        self.addr
    }

    /// Returns the port that the listener is bound to.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Consumes ref, returning inner ref.
    pub fn into_inner(self) -> ServerRef {
        self.inner
    }
}

impl Future for TcpServerRef {
    type Output = Result<(), JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner.task).poll(cx)
    }
}

impl Deref for TcpServerRef {
    type Target = ServerRef;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for TcpServerRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
