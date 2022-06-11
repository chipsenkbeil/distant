use crate::{Listener, WindowsPipeStream};
use async_trait::async_trait;
use std::{
    ffi::{OsStr, OsString},
    fmt, io, mem,
};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

/// Represents a listener for incoming connections over a named windows pipe
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
    type Output = WindowsPipeStream;

    async fn accept(&mut self) -> io::Result<Self::Output> {
        // Wait for a new connection on the current server pipe
        self.inner.connect().await?;

        // Create a new server pipe to use for the next connection
        // as the current pipe is now taken with our existing connection
        let pipe = mem::replace(&mut self.inner, ServerOptions::new().create(&self.addr)?);

        Ok(WindowsPipeStream {
            addr: self.addr.clone(),
            inner: pipe
        })
    }
}
