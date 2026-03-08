use std::path::{Path, PathBuf};
use std::{fmt, io};

use tokio::net::UnixStream;

use super::Listener;
use crate::net::common::UnixSocketTransport;

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
    /// exists. Sets the unix socket file permissions to `mode` atomically during bind by
    /// temporarily adjusting the process umask, eliminating the window where the socket
    /// would be world-accessible.
    pub async fn bind_with_permissions(path: impl AsRef<Path>, mode: u32) -> io::Result<Self> {
        let path = path.as_ref();

        // If the path already exists, check whether something is listening
        if path.exists() {
            if UnixStream::connect(path).await.is_ok() {
                return Err(io::Error::from(io::ErrorKind::AddrInUse));
            }
            // Stale socket file — remove it
            tokio::fs::remove_file(path).await?;
        }

        // Create a raw Unix stream socket via socket2
        let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;

        // Set umask so that bind() creates the socket file with the desired permissions.
        // The kernel applies: actual_mode = 0o777 & !umask, so umask = !mode & 0o777.
        // This eliminates the race window where the socket would be world-accessible
        // between bind() and a post-bind chmod() (fixes #111).
        let desired_umask = !mode & 0o777;
        let old_umask = unsafe { libc::umask(desired_umask as libc::mode_t) };

        let bind_result = socket.bind(&socket2::SockAddr::unix(path)?);

        // Restore umask immediately after bind, regardless of success or failure
        unsafe { libc::umask(old_umask) };

        bind_result?;
        socket.listen(128)?;
        socket.set_nonblocking(true)?;

        // Convert to tokio's UnixListener
        let std_listener: std::os::unix::net::UnixListener = socket.into();
        let listener = tokio::net::UnixListener::from_std(std_listener)?;

        Ok(Self {
            path: path.to_path_buf(),
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
    use std::os::unix::fs::PermissionsExt;

    use tempfile::NamedTempFile;
    use test_log::test;
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;

    use super::*;
    use crate::net::common::TransportExt;

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
                .map_err(|x| io::Error::other(x.display().to_string()))?;

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

    #[test(tokio::test)]
    async fn bind_with_permissions_should_set_socket_file_mode() {
        let path = NamedTempFile::new()
            .expect("Failed to create socket file")
            .path()
            .to_path_buf();

        let _listener = UnixSocketListener::bind_with_permissions(&path, 0o600)
            .await
            .expect("Failed to bind with permissions");

        let metadata = tokio::fs::metadata(&path).await.expect("Failed to stat");
        let actual_mode = metadata.permissions().mode() & 0o777;
        assert_eq!(
            actual_mode, 0o600,
            "Socket file permissions should be 0o600, got 0o{actual_mode:o}"
        );
    }
}
