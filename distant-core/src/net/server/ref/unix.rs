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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    fn make_server_ref() -> ServerRef {
        let (shutdown, _) = broadcast::channel(1);
        let task = tokio::spawn(async {});
        ServerRef { shutdown, task }
    }

    #[test_log::test(tokio::test)]
    async fn new_stores_path() {
        let path = PathBuf::from("/tmp/test.sock");
        let unix_ref = UnixSocketServerRef::new(path.clone(), make_server_ref());
        assert_eq!(unix_ref.path(), Path::new("/tmp/test.sock"));
    }

    #[test_log::test(tokio::test)]
    async fn path_returns_correct_value() {
        let path = PathBuf::from("/var/run/distant/server.sock");
        let unix_ref = UnixSocketServerRef::new(path, make_server_ref());
        assert_eq!(unix_ref.path(), Path::new("/var/run/distant/server.sock"));
    }

    #[test_log::test(tokio::test)]
    async fn into_inner_returns_server_ref() {
        let path = PathBuf::from("/tmp/test.sock");
        let unix_ref = UnixSocketServerRef::new(path, make_server_ref());
        let recovered = unix_ref.into_inner();
        // Let the spawned empty task complete
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(recovered.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn deref_delegates_to_inner_server_ref() {
        let path = PathBuf::from("/tmp/test.sock");
        let unix_ref = UnixSocketServerRef::new(path, make_server_ref());
        // Deref gives us access to ServerRef methods
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(unix_ref.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn deref_mut_delegates_to_inner_server_ref() {
        let path = PathBuf::from("/tmp/test.sock");
        let mut unix_ref = UnixSocketServerRef::new(path, make_server_ref());
        let inner: &mut ServerRef = &mut unix_ref;
        inner.shutdown();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(unix_ref.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn shutdown_via_deref_stops_server() {
        let path = PathBuf::from("/tmp/test.sock");
        let unix_ref = UnixSocketServerRef::new(path, make_server_ref());
        unix_ref.shutdown();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(unix_ref.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn path_with_relative_path() {
        let path = PathBuf::from("relative/path.sock");
        let unix_ref = UnixSocketServerRef::new(path, make_server_ref());
        assert_eq!(unix_ref.path(), Path::new("relative/path.sock"));
    }
}
