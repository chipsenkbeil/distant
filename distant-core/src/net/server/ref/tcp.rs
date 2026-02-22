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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    use tokio::sync::broadcast;

    fn make_server_ref() -> ServerRef {
        let (shutdown, _) = broadcast::channel(1);
        let task = tokio::spawn(async {});
        ServerRef { shutdown, task }
    }

    #[test_log::test(tokio::test)]
    async fn new_stores_addr_and_port() {
        let addr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let port = 8080;
        let tcp_ref = TcpServerRef::new(addr, port, make_server_ref());
        assert_eq!(tcp_ref.ip_addr(), addr);
        assert_eq!(tcp_ref.port(), port);
    }

    #[test_log::test(tokio::test)]
    async fn ip_addr_returns_ipv6_address() {
        let addr = IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1));
        let tcp_ref = TcpServerRef::new(addr, 3000, make_server_ref());
        assert_eq!(tcp_ref.ip_addr(), addr);
    }

    #[test_log::test(tokio::test)]
    async fn port_returns_correct_value() {
        let tcp_ref = TcpServerRef::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 12345, make_server_ref());
        assert_eq!(tcp_ref.port(), 12345);
    }

    #[test_log::test(tokio::test)]
    async fn into_inner_returns_server_ref() {
        let (shutdown_tx, _) = broadcast::channel(1);
        let task = tokio::spawn(async {});
        let inner = ServerRef {
            shutdown: shutdown_tx.clone(),
            task,
        };
        let tcp_ref = TcpServerRef::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9999, inner);
        let recovered = tcp_ref.into_inner();
        // Let the spawned empty task complete
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(recovered.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn deref_delegates_to_inner_server_ref() {
        let tcp_ref = TcpServerRef::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080, make_server_ref());
        // Deref gives us access to ServerRef methods like is_finished and shutdown
        // The spawned empty task should finish quickly
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(tcp_ref.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn deref_mut_delegates_to_inner_server_ref() {
        let mut tcp_ref =
            TcpServerRef::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080, make_server_ref());
        // DerefMut allows mutable access to the inner ServerRef
        let inner: &mut ServerRef = &mut tcp_ref;
        inner.shutdown();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(tcp_ref.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn shutdown_via_deref_stops_server() {
        let tcp_ref = TcpServerRef::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080, make_server_ref());
        tcp_ref.shutdown();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(tcp_ref.is_finished());
    }
}
