use crate::net::{Codec, DataStream, Transport};
use tokio::{
    io,
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream as TokioTcpStream, ToSocketAddrs,
    },
};

impl_async_newtype!(TcpStream -> TokioTcpStream);

impl DataStream for TcpStream {
    type Read = OwnedReadHalf;
    type Write = OwnedWriteHalf;

    fn into_split(self) -> (Self::Read, Self::Write) {
        TokioTcpStream::into_split(self.0)
    }
}

impl<U: Codec> Transport<TcpStream, U> {
    /// Establishes a connection to one of the specified addresses and uses the provided codec
    /// for transportation
    pub async fn connect(addrs: impl ToSocketAddrs, codec: U) -> io::Result<Self> {
        let stream = TokioTcpStream::connect(addrs).await?;
        Ok(Transport::new(TcpStream(stream), codec))
    }
}
