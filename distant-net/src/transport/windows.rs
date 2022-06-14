use crate::{IntoSplit, RawTransport, RawTransportRead, RawTransportWrite};
use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf, ReadHalf, WriteHalf},
    net::windows::named_pipe::ClientOptions,
};

// Equivalent to winapi::shared::winerror::ERROR_PIPE_BUSY
// DWORD -> c_uLong -> u32
const ERROR_PIPE_BUSY: u32 = 231;

// Time between attempts to connect to a busy pipe
const BUSY_PIPE_SLEEP_MILLIS: u64 = 50;

mod pipe;
pub use pipe::NamedPipe;

/// Represents a data stream for a Windows pipe (client or server)
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

        let pipe = loop {
            match ClientOptions::new().open(&addr) {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => (),
                Err(e) => return Err(e),
            }

            tokio::time::sleep(Duration::from_millis(BUSY_PIPE_SLEEP_MILLIS)).await;
        };

        Ok(Self {
            addr,
            inner: NamedPipe::from(pipe),
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

impl RawTransport for WindowsPipeTransport {
    type ReadHalf = ReadHalf<WindowsPipeTransport>;
    type WriteHalf = WriteHalf<WindowsPipeTransport>;
}
impl RawTransportRead for WindowsPipeTransport {}
impl RawTransportWrite for WindowsPipeTransport {}

impl RawTransportRead for ReadHalf<WindowsPipeTransport> {}
impl RawTransportWrite for WriteHalf<WindowsPipeTransport> {}

impl IntoSplit for WindowsPipeTransport {
    type Read = ReadHalf<WindowsPipeTransport>;
    type Write = WriteHalf<WindowsPipeTransport>;

    fn into_split(self) -> (Self::Write, Self::Read) {
        tokio::io::split(self)
    }
}

impl AsyncRead for WindowsPipeTransport {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for WindowsPipeTransport {
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
