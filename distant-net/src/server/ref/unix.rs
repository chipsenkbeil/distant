use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::task::JoinError;

use super::ServerRef;

/// Reference to a unix socket server instance.
pub struct UnixSocketServerRef {
    pub(crate) path: PathBuf,
    pub(crate) inner: ServerRef,
}

impl UnixSocketServerRef {
    pub fn new(path: PathBuf, inner: ServerRef) -> Self {
        Self { path, inner }
    }

    /// Returns the path to the socket.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Consumes ref, returning inner ref.
    pub fn into_inner(self) -> ServerRef {
        self.inner
    }
}

impl Future for UnixSocketServerRef {
    type Output = Result<(), JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner.task).poll(cx)
    }
}

impl Deref for UnixSocketServerRef {
    type Target = ServerRef;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for UnixSocketServerRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
