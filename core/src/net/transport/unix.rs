use super::{Codec, DataStream, Transport};
use tokio::{
    io,
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf, SocketAddr},
        UnixStream,
    },
};

impl DataStream for UnixStream {
    type Read = OwnedReadHalf;
    type Write = OwnedWriteHalf;

    fn to_connection_tag(&self) -> String {
        self.peer_addr()
            .map(|addr| format!("{:?}", addr))
            .unwrap_or_else(|_| String::from("--"))
    }

    fn into_split(self) -> (Self::Read, Self::Write) {
        UnixStream::into_split(self)
    }
}

impl<U: Codec> Transport<UnixStream, U> {
    /// Establishes a connection to the socket at the specified path and uses the provided codec
    /// for transportation
    pub async fn connect(path: impl AsRef<std::path::Path>, codec: U) -> io::Result<Self> {
        let stream = UnixStream::connect(path.as_ref()).await?;
        Ok(Transport::new(stream, codec))
    }

    /// Returns the address of the peer the transport is connected to
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.0.get_ref().peer_addr()
    }
}
