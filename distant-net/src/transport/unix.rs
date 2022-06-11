use crate::Transport;
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
pub struct UnixSocketTransport {
    pub(crate) path: PathBuf,
    pub(crate) inner: UnixStream,
}

impl UnixSocketTransport {
    /// Creates a new stream by connecting to the specified path
    pub async fn connect(path: impl AsRef<Path>) -> io::Result<Self> {
        let stream = UnixStream::connect(path.as_ref()).await?;
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

impl fmt::Debug for UnixSocketTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnixSocketTransport")
            .field("path", &self.path)
            .finish()
    }
}

impl Transport for UnixSocketTransport {
    type ReadHalf = OwnedReadHalf;
    type WriteHalf = OwnedWriteHalf;

    fn into_split(self) -> (Self::ReadHalf, Self::WriteHalf) {
        UnixStream::into_split(self.inner)
    }
}

impl AsyncRead for UnixSocketTransport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for UnixSocketTransport {
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
