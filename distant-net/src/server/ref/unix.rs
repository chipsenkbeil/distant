use super::ServerRef;
use std::path::{Path, PathBuf};

/// Reference to a unix socket server instance
pub struct UnixSocketServerRef {
    pub(crate) path: PathBuf,
    pub(crate) inner: Box<dyn ServerRef>,
}

impl UnixSocketServerRef {
    pub fn new(path: PathBuf, inner: Box<dyn ServerRef>) -> Self {
        Self { path, inner }
    }

    /// Returns the path to the socket
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Consumes ref, returning inner ref
    pub fn into_inner(self) -> Box<dyn ServerRef> {
        self.inner
    }
}

impl ServerRef for UnixSocketServerRef {
    fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    fn shutdown(&self) {
        self.inner.shutdown();
    }
}
