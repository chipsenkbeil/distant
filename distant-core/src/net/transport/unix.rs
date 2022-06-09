use crate::net::{Codec, DataStream, Transport};
use derive_more::{Deref, DerefMut, From};
use tokio::{
    io,
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf},
        UnixStream as TokioUnixStream,
    },
};

#[derive(Deref, DerefMut, From)]
pub struct UnixStream(TokioUnixStream);

impl DataStream for UnixStream {
    type Read = OwnedReadHalf;
    type Write = OwnedWriteHalf;

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
}
