use std::io;
use std::path::PathBuf;

use async_trait::async_trait;

use super::Connector;
use crate::common::UnixSocketTransport;

/// Implementation of [`Connector`] to support connecting via a Unix socket.
pub struct UnixSocketConnector {
    path: PathBuf,
}

impl UnixSocketConnector {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl<T: Into<PathBuf>> From<T> for UnixSocketConnector {
    fn from(path: T) -> Self {
        Self::new(path)
    }
}

#[async_trait]
impl Connector for UnixSocketConnector {
    type Transport = UnixSocketTransport;

    async fn connect(self) -> io::Result<Self::Transport> {
        UnixSocketTransport::connect(self.path).await
    }
}
