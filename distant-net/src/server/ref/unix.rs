use crate::ServerRef;
use std::path::{Path, PathBuf};

/// Reference to a unix socket server instance
pub struct UnixSocketServerRef {
    pub(crate) path: PathBuf,
    pub(crate) inner: Box<dyn ServerRef>,
}

impl UnixSocketServerRef {
    /// Returns the path to the socket
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ServerRef for UnixSocketServerRef {
    fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    fn abort(&self) {
        self.inner.abort();
    }
}
