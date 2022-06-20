use crate::{IntoSplit, RawTransport, RawTransportRead, RawTransportWrite};
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

/// Represents a [`RawTransport`] that leverages a Unix socket
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

impl RawTransport for UnixSocketTransport {
    type ReadHalf = OwnedReadHalf;
    type WriteHalf = OwnedWriteHalf;
}
impl RawTransportRead for UnixSocketTransport {}
impl RawTransportWrite for UnixSocketTransport {}

impl RawTransportRead for OwnedReadHalf {}
impl RawTransportWrite for OwnedWriteHalf {}

impl IntoSplit for UnixSocketTransport {
    type Read = OwnedReadHalf;
    type Write = OwnedWriteHalf;

    fn into_split(self) -> (Self::Write, Self::Read) {
        let (r, w) = UnixStream::into_split(self.inner);
        (w, r)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::UnixListener,
        sync::oneshot,
        task::JoinHandle,
    };

    #[tokio::test]
    async fn should_fail_to_connect_if_socket_does_not_exist() {
        // Generate a socket path and delete the file after so there is nothing there
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .path()
            .to_path_buf();

        // Now this should fail as we're already bound to the name
        UnixSocketTransport::connect(&path)
            .await
            .expect_err("Unexpectedly succeeded in connecting to missing socket");
    }

    #[tokio::test]
    async fn should_fail_to_connect_if_path_is_not_a_socket() {
        // Generate a regular file
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .into_temp_path();

        // Now this should fail as this file is not a socket
        UnixSocketTransport::connect(&path)
            .await
            .expect_err("Unexpectedly succeeded in connecting to regular file");
    }

    #[tokio::test]
    async fn should_be_able_to_send_and_receive_data() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for a connection, send data,
        // and receive data that it will return in the task
        let task: JoinHandle<io::Result<()>> = tokio::spawn(async move {
            // Generate a socket path and delete the file after so there is nothing there
            let path = NamedTempFile::new()
                .expect("Failed to create socket file")
                .path()
                .to_path_buf();

            // Start listening at the socket path
            let socket = UnixListener::bind(&path)?;

            // Send the path back to our main test thread
            tx.send(path)
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x.display().to_string()))?;

            // Get the connection
            let (mut conn, _) = socket.accept().await?;

            // Send some data to the connection (10 bytes)
            conn.write_all(b"hello conn").await?;

            // Receive some data from the connection (12 bytes)
            let mut buf: [u8; 12] = [0; 12];
            let _ = conn.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"hello server");

            Ok(())
        });

        // Wait for the server to be ready
        let path = rx.await.expect("Failed to get server socket path");

        // Connect to the socket, send some bytes, and get some bytes
        let mut buf: [u8; 10] = [0; 10];

        let mut conn = UnixSocketTransport::connect(&path)
            .await
            .expect("Conn failed to connect");
        conn.read_exact(&mut buf)
            .await
            .expect("Conn failed to read");
        assert_eq!(&buf, b"hello conn");

        conn.write_all(b"hello server")
            .await
            .expect("Conn failed to write");

        // Verify that the task has completed by waiting on it
        let _ = task.await.expect("Server task failed unexpectedly");
    }
}
