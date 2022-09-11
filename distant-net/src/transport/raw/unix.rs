use super::{Interest, RawTransport, Ready, Reconnectable};
use async_trait::async_trait;
use std::{
    fmt, io,
    path::{Path, PathBuf},
};
use tokio::net::UnixStream;

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

#[async_trait]
impl Reconnectable for UnixSocketTransport {
    async fn reconnect(&mut self) -> io::Result<()> {
        self.inner = UnixStream::connect(self.path.as_path()).await?;
        Ok(())
    }
}

#[async_trait]
impl RawTransport for UnixSocketTransport {
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.try_read(buf)
    }

    fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.try_write(buf)
    }

    async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        self.inner.ready(interest).await
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
