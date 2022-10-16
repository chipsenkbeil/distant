use super::Listener;
use crate::common::{NamedPipe, WindowsPipeTransport};
use async_trait::async_trait;
use std::{
    ffi::{OsStr, OsString},
    fmt, io, mem,
};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

/// Represents a [`Listener`] for incoming connections over a named windows pipe
pub struct WindowsPipeListener {
    addr: OsString,
    inner: NamedPipeServer,
}

impl WindowsPipeListener {
    /// Creates a new listener by binding to the specified local address
    /// using the given name, which translates to `\\.\pipe\{name}`
    pub fn bind_local(name: impl AsRef<OsStr>) -> io::Result<Self> {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::bind(addr)
    }

    /// Creates a new listener by binding to the specified address
    pub fn bind(addr: impl Into<OsString>) -> io::Result<Self> {
        let addr = addr.into();
        let pipe = ServerOptions::new()
            .first_pipe_instance(true)
            .create(addr.as_os_str())?;
        Ok(Self { addr, inner: pipe })
    }

    /// Returns the addr that the listener is bound to
    pub fn addr(&self) -> &OsStr {
        &self.addr
    }
}

impl fmt::Debug for WindowsPipeListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowsPipeListener")
            .field("addr", &self.addr)
            .finish()
    }
}

#[async_trait]
impl Listener for WindowsPipeListener {
    type Output = WindowsPipeTransport;

    async fn accept(&mut self) -> io::Result<Self::Output> {
        // Wait for a new connection on the current server pipe
        self.inner.connect().await?;

        // Create a new server pipe to use for the next connection
        // as the current pipe is now taken with our existing connection
        let pipe = mem::replace(&mut self.inner, ServerOptions::new().create(&self.addr)?);

        Ok(WindowsPipeTransport {
            addr: self.addr.clone(),
            inner: NamedPipe::from(pipe),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Transport;
    use test_log::test;
    use tokio::{sync::oneshot, task::JoinHandle};

    #[test(tokio::test)]
    async fn should_fail_to_bind_if_pipe_already_bound() {
        // Generate a pipe name
        let name = format!("test_pipe_{}", rand::random::<usize>());

        // Listen at the pipe
        let _listener =
            WindowsPipeListener::bind_local(&name).expect("Unexpectedly failed to bind first time");

        // Now this should fail as we're already bound to the name
        WindowsPipeListener::bind_local(&name)
            .expect_err("Unexpectedly succeeded in binding to same pipe");
    }

    #[test(tokio::test)]
    async fn should_be_able_to_receive_connections_and_read_and_write_data_with_them() {
        let (tx, rx) = oneshot::channel();

        // Spawn a task that will wait for two connections and then
        // return the success or failure
        let task: JoinHandle<io::Result<()>> = tokio::spawn(async move {
            // Generate a pipe name
            let name = format!("test_pipe_{}", rand::random::<usize>());

            // Listen at the pipe
            let mut listener = WindowsPipeListener::bind_local(&name)?;

            // Send the name back to our main test thread
            tx.send(name)
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

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
        let name = rx.await.expect("Failed to get server name");

        // Connect to the listener twice, sending some bytes and receiving some bytes from each
        let mut buf: [u8; 12] = [0; 12];

        let conn = WindowsPipeTransport::connect_local(&name)
            .await
            .expect("Conn 1 failed to connect");
        conn.write_all(b"hello server 1")
            .await
            .expect("Conn 1 failed to write");
        conn.read_exact(&mut buf)
            .await
            .expect("Conn 1 failed to read");
        assert_eq!(&buf, b"hello conn 1");

        let conn = WindowsPipeTransport::connect_local(&name)
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
