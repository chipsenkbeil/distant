use std::future::Future;
use std::sync::Arc;

use log::error;
use tokio::runtime::Handle;
use tokio::sync::{OnceCell, watch};

use super::config::MountConfig;
use super::remote::RemoteFs;
use distant_core::Channel;

/// Async-to-sync bridge for mount backends.
///
/// Backends that use synchronous callbacks (FUSE, FileProvider, CloudFiles)
/// use [`spawn`](Self::spawn) to dispatch async work against the [`RemoteFs`].
/// The `RemoteFs` may be provided up-front ([`with_fs`](Self::with_fs)) or
/// initialised lazily from a future ([`new`](Self::new)).
#[allow(dead_code)]
pub(crate) struct Runtime {
    handle: Handle,
    fs: Arc<OnceCell<Arc<RemoteFs>>>,
    ready: watch::Receiver<bool>,
}

#[allow(dead_code)]
impl Runtime {
    /// Creates a runtime that lazily initialises `RemoteFs` from the given
    /// future. [`spawn`](Self::spawn) calls will wait until init completes.
    #[allow(dead_code)]
    pub fn new<F>(handle: Handle, init: F) -> Self
    where
        F: Future<Output = (Channel, MountConfig)> + Send + 'static,
    {
        let fs = Arc::new(OnceCell::new());
        let (tx, rx) = watch::channel(false);
        let fs_clone = Arc::clone(&fs);

        handle.spawn(async move {
            let (channel, config) = init.await;
            match RemoteFs::init(channel, config).await {
                Ok(remote_fs) => {
                    let _ = fs_clone.set(Arc::new(remote_fs));
                    let _ = tx.send(true);
                }
                Err(e) => {
                    error!("failed to initialize RemoteFs: {e}");
                    // tx is dropped without sending true — spawn() callers
                    // will see the channel close and log an error.
                }
            }
        });

        Self {
            handle,
            fs,
            ready: rx,
        }
    }

    /// Creates a runtime with a pre-initialised `RemoteFs` (normal mount path).
    pub fn with_fs(handle: Handle, fs: RemoteFs) -> Self {
        let cell = Arc::new(OnceCell::new());
        let _ = cell.set(Arc::new(fs));
        let (_tx, rx) = watch::channel(true);
        Self {
            handle,
            fs: cell,
            ready: rx,
        }
    }

    /// Spawns async work that waits for init, then runs with an `Arc<RemoteFs>`.
    pub fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Arc<RemoteFs>) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let fs = Arc::clone(&self.fs);
        let mut ready = self.ready.clone();
        self.handle.spawn(async move {
            if ready.wait_for(|v| *v).await.is_err() {
                error!("runtime init failed, cannot execute operation");
                return;
            }
            let fs = Arc::clone(fs.get().expect("ready signaled but fs not set"));
            f(fs).await;
        });
    }
}
