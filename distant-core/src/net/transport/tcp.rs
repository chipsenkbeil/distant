use crate::net::{Codec, DataStream, Transport};
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
}
