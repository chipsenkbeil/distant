use crate::{Listener, UnixSocketTransport};
use async_trait::async_trait;
use std::{
    fmt, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use tokio::net::{UnixListener, UnixStream};

/// Represents a [`Listener`] for incoming connections over a Unix socket
pub struct UnixSocketListener {
    path: PathBuf,
    inner: tokio::net::UnixListener,
}

impl UnixSocketListener {
    /// Creates a new listener by binding to the specified path, failing if the path already
    /// exists. Sets permission of unix socket to `0o600` where only the owner can read from and
    /// write to the socket.
    pub async fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::bind_with_permissions(path, Self::default_unix_socket_file_permissions()).await
    }

    /// Creates a new listener by binding to the specified path, failing if the path already
    /// exists. Sets the unix socket file permissions to `mode`.
    pub async fn bind_with_permissions(path: impl AsRef<Path>, mode: u32) -> io::Result<Self> {
        // Attempt to bind to the path, and if we fail, we see if we can connect
        // to the path -- if not, we can try to delete the path and start again
        let listener = match UnixListener::bind(path.as_ref()) {
            Ok(listener) => listener,
            Err(_) => {
                // If we can connect to the path, then it's already in use
                if UnixStream::connect(path.as_ref()).await.is_ok() {
                    return Err(io::Error::from(io::ErrorKind::AddrInUse));
                }

                // Otherwise, remove the file and try again
                tokio::fs::remove_file(path.as_ref()).await?;

                UnixListener::bind(path.as_ref())?
            }
        };

        // TODO: We should be setting this permission during bind, but neither std library nor
        //       tokio have support for this. We would need to create our own raw socket and
        //       use libc to change the permissions via the raw file descriptor
        //
        // See https://github.com/chipsenkbeil/distant/issues/111
        let mut permissions = tokio::fs::metadata(path.as_ref()).await?.permissions();
        permissions.set_mode(mode);
        tokio::fs::set_permissions(path.as_ref(), permissions).await?;

        Ok(Self {
            path: path.as_ref().to_path_buf(),
            inner: listener,
        })
    }

    /// Returns the path to the socket
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the default unix socket file permissions as an octal (e.g. `0o600`)
    pub const fn default_unix_socket_file_permissions() -> u32 {
        0o600
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
    use crate::Transport;
    use tempfile::NamedTempFile;
    use test_log::test;
    use tokio::{sync::oneshot, task::JoinHandle};

    #[test(tokio::test)]
    async fn should_succeed_to_bind_if_file_exists_at_path_but_nothing_listening() {
        // Generate a socket path
        let path = NamedTempFile::new()
            .expect("Failed to create file")
            .into_temp_path();

        // This should fail as we're already got a file at the path
        UnixSocketListener::bind(&path)
            .await
            .expect("Unexpectedly failed to bind to existing file");
    }

    #[test(tokio::test)]
    async fn should_fail_to_bind_if_socket_already_bound() {
        // Generate a socket path and delete the file after
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .path()
            .to_path_buf();

        // Listen at the socket
        let _listener = UnixSocketListener::bind(&path)
            .await
            .expect("Unexpectedly failed to bind first time");

        // Now this should fail as we're already bound to the path
        UnixSocketListener::bind(&path)
            .await
            .expect_err("Unexpectedly succeeded in binding to same socket");
    }

    #[test(tokio::test)]
    async fn should_be_able_to_receive_connections_and_read_and_write_data_with_them() {
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
            let mut listener = UnixSocketListener::bind(&path).await?;

            // Send the name path to our main test thread
            tx.send(path)
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x.display().to_string()))?;

            // Get first connection
            let conn_1 = listener.accept().await?;

            // Send some data to the first connection (12 bytes)
            conn_1.write_all(b"hello conn 1").await?;

            // Get some data from the first connection (14 bytes)
            let mut buf: [u8; 14] = [0; 14];
            let _ = conn_1.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"hello server 1");

            // Get second connection
            let conn_2 = listener.accept().await?;

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

        let conn = UnixSocketTransport::connect(&path)
            .await
            .expect("Conn 1 failed to connect");
        conn.write_all(b"hello server 1")
            .await
            .expect("Conn 1 failed to write");
        conn.read_exact(&mut buf)
            .await
            .expect("Conn 1 failed to read");
        assert_eq!(&buf, b"hello conn 1");

        let conn = UnixSocketTransport::connect(&path)
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
