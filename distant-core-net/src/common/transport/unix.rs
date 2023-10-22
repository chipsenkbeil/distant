use std::path::{Path, PathBuf};
use std::{fmt, io};

use async_trait::async_trait;
use tokio::net::UnixStream;

use super::{Interest, Ready, Reconnectable, Transport};

/// Represents a [`Transport`] that leverages a Unix socket
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
impl Transport for UnixSocketTransport {
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
    use tempfile::NamedTempFile;
    use test_log::test;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;

    use super::*;
    use crate::common::TransportExt;

    async fn start_and_run_server(tx: oneshot::Sender<PathBuf>) -> io::Result<()> {
        // Generate a socket path and delete the file after so there is nothing there
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .path()
            .to_path_buf();

        // Start listening at the socket path
        let listener = UnixListener::bind(&path)?;

        // Send the path back to our main test thread
        tx.send(path)
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x.display().to_string()))?;

        run_server(listener).await
    }

    async fn run_server(listener: UnixListener) -> io::Result<()> {
        // Get the connection
        let (mut conn, _) = listener.accept().await?;

        // Send some data to the connection (10 bytes)
        conn.write_all(b"hello conn").await?;

        // Receive some data from the connection (12 bytes)
        let mut buf: [u8; 12] = [0; 12];
        let _ = conn.read_exact(&mut buf).await?;
        assert_eq!(&buf, b"hello server");

        Ok(())
    }

    #[test(tokio::test)]
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

    #[test(tokio::test)]
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

    #[test(tokio::test)]
    async fn should_be_able_to_read_and_write_data() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for a connection, send data,
        // and receive data that it will return in the task
        let task: JoinHandle<io::Result<()>> = tokio::spawn(start_and_run_server(tx));

        // Wait for the server to be ready
        let path = rx.await.expect("Failed to get server socket path");

        // Connect to the socket, send some bytes, and get some bytes
        let mut buf: [u8; 10] = [0; 10];

        let conn = UnixSocketTransport::connect(&path)
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

    #[test(tokio::test)]
    async fn should_be_able_to_reconnect() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for a connection, send data,
        // and receive data that it will return in the task
        let task: JoinHandle<io::Result<()>> = tokio::spawn(start_and_run_server(tx));

        // Wait for the server to be ready
        let path = rx.await.expect("Failed to get server socket path");

        // Connect to the server
        let mut conn = UnixSocketTransport::connect(&path)
            .await
            .expect("Conn failed to connect");

        // Kill the server to make the connection fail
        task.abort();

        // Verify the connection fails by trying to read from it (should get connection reset)
        conn.readable()
            .await
            .expect("Failed to wait for conn to be readable");
        let res = conn.read_exact(&mut [0; 10]).await;
        assert!(
            matches!(res, Ok(0) | Err(_)),
            "Unexpected read result: {res:?}"
        );

        // Restart the server (need to remove the socket file)
        let _ = tokio::fs::remove_file(&path).await;
        let task: JoinHandle<io::Result<()>> = tokio::spawn(run_server(
            UnixListener::bind(&path).expect("Failed to rebind server"),
        ));

        // Reconnect to the socket, send some bytes, and get some bytes
        let mut buf: [u8; 10] = [0; 10];
        conn.reconnect().await.expect("Conn failed to reconnect");

        // Continually read until we get all of the data
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
