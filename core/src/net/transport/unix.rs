use super::{DataStream, SecretKey, Transport};
use std::sync::Arc;
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

impl Transport<UnixStream> {
    /// Establishes a connection using the provided session and performs a handshake to establish
    /// means of encryption, returning a transport ready to communicate with the other side
    ///
    /// Takes an optional authentication key
    pub async fn connect(
        path: impl AsRef<std::path::Path>,
        auth_key: Option<Arc<SecretKey>>,
    ) -> io::Result<Self> {
        let stream = UnixStream::connect(path.as_ref()).await?;
        Self::from_handshake(stream, auth_key).await
    }

    /// Returns the address of the peer the transport is connected to
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.conn.get_ref().peer_addr()
    }
}
