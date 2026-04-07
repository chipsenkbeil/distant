use std::io;

/// Handle to an active filesystem mount.
///
/// Provides methods to gracefully unmount or wait for an externally-driven
/// unmount (e.g. Ctrl+C). Dropping the handle without detaching will result
/// in a call to [`unmount`].
///
/// [`unmount`]: MountHandle::unmount
#[derive(Debug)]
pub struct MountHandle {
    /// Sender half of the shutdown signal. Consumed by [`unmount`] or [`Drop`].
    ///
    /// [`unmount`]: MountHandle::unmount
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,

    /// Join handle for the background mount task. Consumed by [`unmount`] or
    /// [`wait`].
    ///
    /// [`unmount`]: MountHandle::unmount
    /// [`wait`]: MountHandle::wait
    join_handle: Option<tokio::task::JoinHandle<io::Result<()>>>,

    /// Whether to unmount when the handle is dropped.
    unmount_on_drop: bool,
}

impl MountHandle {
    /// Creates a new handle to the mounted filesystem.
    pub(crate) fn new(
        shutdown_tx: tokio::sync::oneshot::Sender<()>,
        join_handle: tokio::task::JoinHandle<io::Result<()>>,
    ) -> Self {
        Self {
            shutdown_tx: Some(shutdown_tx),
            join_handle: Some(join_handle),
            unmount_on_drop: true,
        }
    }

    /// Returns `true` while the background mount task has not yet
    /// completed. Used by [`MountHandle::probe`](distant_core::plugin::MountHandle::probe)
    /// implementations on the wrapper to detect outer-task death
    /// (panic or premature exit).
    ///
    /// Note: this is a coarse signal. The outer task only ends after
    /// the inner backend (NFS server / FUSE BackgroundSession /
    /// FP polling loop / WCF watcher) has been told to shut down,
    /// so this returns `true` for the entire normal lifetime of the
    /// mount and only flips to `false` on panic or completion.
    /// More granular per-backend liveness checks can be layered on
    /// top in the wrapper.
    pub(crate) fn is_alive(&self) -> bool {
        self.join_handle
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }

    /// Detaches the handle, meaning that dropping it will no longer unmount.
    pub(crate) fn detach(mut self) -> Self {
        self.unmount_on_drop = false;
        self
    }

    /// Attempts to unmount the filesystem, waiting until complete.
    pub(crate) async fn unmount(mut self) -> io::Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            // The receiver may already be dropped if the mount exited on its
            // own; that is not an error.
            let _ = tx.send(());
        }

        self.wait().await
    }

    /// Waits until the filesystem is unmounted.
    pub(crate) async fn wait(mut self) -> io::Result<()> {
        match self.join_handle.take() {
            Some(handle) => handle
                .await
                .unwrap_or_else(|err| Err(io::Error::other(err))),
            None => Ok(()),
        }
    }
}

impl Drop for MountHandle {
    /// Unless detached, dropping the handle will result in attempting to unmount the filesystem.
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take()
            && self.unmount_on_drop
        {
            let _ = tx.send(());
        }
    }
}
