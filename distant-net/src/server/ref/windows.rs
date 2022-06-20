use crate::{ServerRef, ServerState};
use std::ffi::{OsStr, OsString};

/// Reference to a unix socket server instance
pub struct WindowsPipeServerRef {
    pub(crate) addr: OsString,
    pub(crate) inner: Box<dyn ServerRef>,
}

impl WindowsPipeServerRef {
    /// Returns the addr that the listener is bound to
    pub fn addr(&self) -> &OsStr {
        &self.addr
    }
}

impl ServerRef for WindowsPipeServerRef {
    fn state(&self) -> &ServerState {
        self.inner.state()
    }

    fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    fn abort(&self) {
        self.inner.abort();
    }
}
