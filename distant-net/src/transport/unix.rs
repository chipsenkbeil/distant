use crate::DataStream;
use std::{
    fmt, io,
    path::{Path, PathBuf},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf},
        UnixStream,
    },
};

/// Represents a data stream for a Unix socket
pub struct UnixSocketStream {
    pub(crate) path: PathBuf,
    pub(crate) inner: UnixStream,
}

impl UnixSocketStream {
    /// Creates a new stream by connecting to the specified path
    pub async fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        let stream = tokio::net::UnixStream::connect(path.as_ref()).await?;
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            inner: stream,
        })
    }

    /// Returns the path to the socket
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Debug for UnixSocketStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnixSocketStream")
            .field("path", &self.path)
            .finish()
    }
}

impl DataStream for UnixSocketStream {
    type Read = OwnedReadHalf;
    type Write = OwnedWriteHalf;

    fn into_split(self) -> (Self::Read, Self::Write) {
        tokio::net::UnixStream::into_split(self.inner)
    }
}

impl AsyncRead for UnixSocketStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for UnixSocketStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
