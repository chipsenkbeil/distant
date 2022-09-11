use super::{Interest, RawTransport, Ready, Reconnectable};
use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

mod pipe;
pub use pipe::NamedPipe;

/// Represents a [`RawTransport`] that leverages a named Windows pipe (client or server)
pub struct WindowsPipeTransport {
    pub(crate) addr: OsString,
    pub(crate) inner: NamedPipe,
}

impl WindowsPipeTransport {
    /// Establishes a connection to the pipe with the specified name, using the
    /// name for a local pipe address in the form of `\\.\pipe\my_pipe_name` where
    /// `my_pipe_name` is provided to this function
    pub async fn connect_local(name: impl AsRef<OsStr>) -> io::Result<Self> {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::connect(addr).await
    }

    /// Establishes a connection to the pipe at the specified address
    ///
    /// Address may be something like `\.\pipe\my_pipe_name`
    pub async fn connect(addr: impl Into<OsString>) -> io::Result<Self> {
        let addr = addr.into();
        let inner = NamedPipe::connect_as_client(&addr).await?;

        Ok(Self {
            addr,
            inner,
        })
    }

    /// Returns the addr that the listener is bound to
    pub fn addr(&self) -> &OsStr {
        &self.addr
    }
}

impl fmt::Debug for WindowsPipeTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowsPipeTransport")
            .field("addr", &self.addr)
            .finish()
    }
}

#[async_trait]
impl Reconnectable for WindowsPipeTransport {
    async fn reconnect(&mut self) -> io::Result<()> {
        // We cannot reconnect from server-side
        if self.inner.is_server() {
            return Err(io::Error::from(io::ErrorKind::Unsupported));
        }

        // Drop the existing connection to ensure we are disconnected before trying again
        drop(self.inner);

        self.inner = NamedPipe::connect_as_client(&self.addr).await?;
        Ok(())
    }
}

#[async_trait]
impl RawTransport for WindowsPipeTransport {
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
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::windows::named_pipe::ServerOptions,
        sync::oneshot,
        task::JoinHandle,
    };

    #[tokio::test]
    async fn should_fail_to_connect_if_pipe_does_not_exist() {
        // Generate a pipe name
        let name = format!("test_pipe_{}", rand::random::<usize>());

        // Now this should fail as we're already bound to the name
        WindowsPipeTransport::connect_local(&name)
            .await
            .expect_err("Unexpectedly succeeded in connecting to missing pipe");
    }

    #[tokio::test]
    async fn should_be_able_to_send_and_receive_data() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for a connection, send data,
        // and receive data that it will return in the task
        let task: JoinHandle<io::Result<()>> = tokio::spawn(async move {
            // Generate a pipe address (not just a name)
            let addr = format!(r"\\.\pipe\test_pipe_{}", rand::random::<usize>());

            // Listen at the pipe
            let pipe = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&addr)?;

            // Send the address back to our main test thread
            tx.send(addr)
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

            // Get the connection
            let mut conn = {
                pipe.connect().await?;
                pipe
            };

            // Send some data to the connection (10 bytes)
            conn.write_all(b"hello conn").await?;

            // Receive some data from the connection (12 bytes)
            let mut buf: [u8; 12] = [0; 12];
            let _ = conn.read_exact(&mut buf).await?;
            assert_eq!(&buf, b"hello server");

            Ok(())
        });

        // Wait for the server to be ready
        let address = rx.await.expect("Failed to get server address");

        // Connect to the pipe, send some bytes, and get some bytes
        let mut buf: [u8; 10] = [0; 10];

        let mut conn = WindowsPipeTransport::connect(&address)
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
