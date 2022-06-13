use crate::{Listener, UnixSocketTransport};
use async_trait::async_trait;
use std::{
    fmt, io,
    path::{Path, PathBuf},
};

/// Represents a listener for incoming connections over a Unix socket
pub struct UnixSocketListener {
    path: PathBuf,
    inner: tokio::net::UnixListener,
}

impl UnixSocketListener {
    /// Creates a new listener by binding to the specified path, failing
    /// if the path already exists
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        let listener = tokio::net::UnixListener::bind(path.as_ref())?;
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            inner: listener,
        })
    }

    /// Returns the path to the socket
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Debug for UnixSocketListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UnixSocketListener")
            .field("path", &self.path)
            .finish()
    }
}

#[async_trait]
impl Listener for UnixSocketListener {
    type Output = UnixSocketTransport;

    async fn accept(&mut self) -> io::Result<Self::Output> {
        // NOTE: Address provided is unnamed, or at least the `as_pathname()` method is
        //       returning none, so we use our listener's path, which is the same as
        //       what is being connected, anyway
        let (stream, _) = tokio::net::UnixListener::accept(&self.inner).await?;
        Ok(UnixSocketTransport {
            path: self.path.to_path_buf(),
            inner: stream,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::oneshot,
        task::JoinHandle,
    };

    #[tokio::test]
    async fn should_fail_to_bind_if_file_exists_at_path() {
        // Generate a socket path
        let path = NamedTempFile::new()
            .expect("Failed to create file")
            .into_temp_path();

        // This should fail as we're already got a file at the path
        UnixSocketListener::bind(&path)
            .expect_err("Unexpectedly succeeded in binding to existing file");
    }

    #[tokio::test]
    async fn should_fail_to_bind_if_socket_already_bound() {
        // Generate a socket path and delete the file after
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .path()
            .to_path_buf();

        // Listen at the socket
        let _listener =
            UnixSocketListener::bind(&path).expect("Unexpectedly failed to bind first time");

        // Now this should fail as we're already bound to the path
        UnixSocketListener::bind(&path)
            .expect_err("Unexpectedly succeeded in binding to same socket");
    }

    #[tokio::test]
    async fn should_be_able_to_receive_connections_and_send_and_receive_data_with_them() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for two connections and then
        // return the success or failure
        let task: JoinHandle<io::Result<()>> = tokio::spawn(async move {
            // Generate a socket path and delete the file after
            let path = NamedTempFile::new()
                .expect("Failed to create socket file")
                .path()
                .to_path_buf();

            // Listen at the socket
            let mut listener = UnixSocketListener::bind(&path)?;

            // Send the name path to our main test thread
            tx.send(path)
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x.display().to_string()))?;

            // Get first connection
            let mut conn_1 = listener.accept().await?;

            // Send some data to the first connection (12 bytes)
            conn_1.write_all(b"hello conn 1").await?;

            // Get some data from the first connection (14 bytes)
            let mut buf: [u8; 14] = [0; 14];
            let _ = conn_1.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"hello server 1");

            // Get second connection
            let mut conn_2 = listener.accept().await?;

            // Send some data on to second connection (12 bytes)
            conn_2.write_all(b"hello conn 2").await?;

            // Get some data from the second connection (14 bytes)
            let mut buf: [u8; 14] = [0; 14];
            let _ = conn_2.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"hello server 2");

            Ok(())
        });

        // Wait for the server to be ready
        let path = rx.await.expect("Failed to get server socket path");

        // Connect to the listener twice, sending some bytes and receiving some bytes from each
        let mut buf: [u8; 12] = [0; 12];

        let mut conn = UnixSocketTransport::connect(&path)
            .await
            .expect("Conn 1 failed to connect");
        conn.write_all(b"hello server 1")
            .await
            .expect("Conn 1 failed to write");
        conn.read_exact(&mut buf)
            .await
            .expect("Conn 1 failed to read");
        assert_eq!(&buf, b"hello conn 1");

        let mut conn = UnixSocketTransport::connect(&path)
            .await
            .expect("Conn 2 failed to connect");
        conn.write_all(b"hello server 2")
            .await
            .expect("Conn 2 failed to write");
        conn.read_exact(&mut buf)
            .await
            .expect("Conn 2 failed to read");
        assert_eq!(&buf, b"hello conn 2");

        // Verify that the task has completed by waiting on it
        let _ = task.await.expect("Listener task failed unexpectedly");
    }
}
