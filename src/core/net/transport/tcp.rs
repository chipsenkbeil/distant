use super::{DataStream, SecretKey, Transport};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    io,
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream, ToSocketAddrs,
    },
};

impl DataStream for TcpStream {
    type Read = OwnedReadHalf;
    type Write = OwnedWriteHalf;

    fn to_connection_tag(&self) -> String {
        self.peer_addr()
            .map(|addr| format!("{}", addr))
            .unwrap_or_else(|_| String::from("--"))
    }

    fn into_split(self) -> (Self::Read, Self::Write) {
        TcpStream::into_split(self)
    }
}

impl Transport<TcpStream> {
    /// Establishes a connection using the provided session and performs a handshake to establish
    /// means of encryption, returning a transport ready to communicate with the other side
    ///
    /// Takes an optional authentication key
    pub async fn connect(
        addrs: impl ToSocketAddrs,
        auth_key: Option<Arc<SecretKey>>,
    ) -> io::Result<Self> {
        let stream = TcpStream::connect(addrs).await?;
        Self::from_handshake(stream, auth_key).await
    }

    /// Returns the address of the peer the transport is connected to
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.conn.get_ref().peer_addr()
    }
}
