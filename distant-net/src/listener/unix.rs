use crate::{Listener, UnixSocketStream};
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
    type Output = UnixSocketStream;

    async fn accept(&mut self) -> io::Result<Self::Output> {
        let (stream, addr) = tokio::net::UnixListener::accept(&self.inner).await?;
        Ok(UnixSocketStream {
            path: addr
                .as_pathname()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::Other,
                        "Connected unix socket missing pathname",
                    )
                })?
                .to_path_buf(),
            inner: stream,
        })
    }
}
