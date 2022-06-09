use crate::net::{Codec, DataStream, Transport};
use tokio::{
    io,
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf},
        UnixStream as TokioUnixStream,
    },
};

impl_async_newtype!(UnixSocketStream -> TokioUnixStream);

impl DataStream for UnixSocketStream {
    type Read = OwnedReadHalf;
    type Write = OwnedWriteHalf;

    fn into_split(self) -> (Self::Read, Self::Write) {
        TokioUnixStream::into_split(self.0)
    }
}

impl<U: Codec> Transport<UnixSocketStream, U> {
    /// Establishes a connection to the socket at the specified path and uses the provided codec
    /// for transportation
    pub async fn connect(path: impl AsRef<std::path::Path>, codec: U) -> io::Result<Self> {
        let stream = TokioUnixStream::connect(path.as_ref()).await?;
        Ok(Transport::new(UnixSocketStream(stream), codec))
    }
}
