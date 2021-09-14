use super::{Codec, DataStream, Transport};
use std::net::SocketAddr;
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

impl<U: Codec> Transport<TcpStream, U> {
    /// Establishes a connection to one of the specified addresses and uses the provided codec
    /// for transportation
    pub async fn connect(addrs: impl ToSocketAddrs, codec: U) -> io::Result<Self> {
        let stream = TcpStream::connect(addrs).await?;
        Ok(Transport::new(stream, codec))
    }

    /// Returns the address of the peer the transport is connected to
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.0.get_ref().peer_addr()
    }
}
