use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::task::JoinError;

use super::ServerRef;

/// Reference to a windows pipe server instance.
pub struct WindowsPipeServerRef {
    pub(crate) addr: OsString,
    pub(crate) inner: ServerRef,
}

impl WindowsPipeServerRef {
    pub fn new(addr: OsString, inner: ServerRef) -> Self {
        Self { addr, inner }
    }

    /// Returns the addr that the listener is bound to.
    pub fn addr(&self) -> &OsStr {
        &self.addr
    }

    /// Consumes ref, returning inner ref.
    pub fn into_inner(self) -> ServerRef {
        self.inner
    }
}

impl Future for WindowsPipeServerRef {
    type Output = Result<(), JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner.task).poll(cx)
    }
}

impl Deref for WindowsPipeServerRef {
    type Target = ServerRef;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for WindowsPipeServerRef {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
