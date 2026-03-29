use crate::core::{MountConfig, RemoteFs};
use distant_core::Channel;
use std::io;
use std::sync::Arc;
use std::thread;
use tokio::runtime::{Builder, Runtime as TokioRuntime};
use tokio::sync::OnceCell;

const MAXIMUM_FS_THREADS: usize = 12;
const RUNTIME_CHANNEL_BUFFER_SIZE: usize = 100;
const RUNTIME_EVENT_LOOP_BUFFER_SIZE: usize = 100;

//
// 1. Need to initialize RemoteFs at some point
// 2. Initialize involves passing distant channel
// 3. distant channel may need to connect first
// 4. once initialized, needs to have access to fs across tasks
//
// thoughts are
// need to store an async func to do channel connect
// from there, when we first need to call fs, will connect
// and then proceed from there
//
// or actually we just want to kick off the connection
// and have all of the subsequent spawns wait for the connection
// to be ready or fail if the connection failed
//
pub struct Runtime {
    rt: TokioRuntime,
    fs: Arc<OnceCell<RemoteFs>>,
}

impl Runtime {
    /// Creates a new runtime with an uninitialized filesystem that will attempt to initialize on
    /// first task spawn.
    pub fn new<F, Fut>(f: F) -> io::Result<Self>
    where
        F: Future<Output = io::Result<(Channel, MountConfig)>> + Send + 'static,
    {
        let worker_threads = std::cmp::min(
            MAXIMUM_FS_THREADS,
            thread::available_parallelism().map_or(1, |n| n.get()),
        );

        let rt = Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .enable_all()
            .build()?;

        let fs = Arc::new(OnceCell::new());
        {
            let fs = Arc::clone(&fs);
            rt.spawn(async move {
                fs
                let (channel, config) = f.await.expect("failed to initialize distant channel");
                let fs = RemoteFs::init(channel, config)
                    .await
                    .expect("failed to initialize remote filesystem");
            });
        }

        Ok(Self { rt, fs })
    }

    /// Spawns a new task with a reference to the associated remote filesystem.
    pub fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(&RemoteFs) -> Fut + Send + 'static,
        Fut: Future + Send,
        Fut::Output: Send + 'static,
    {
        let fs = Arc::clone(&self.fs);

        self.rt.spawn(async move {
            let fs_lock = fs.read().await;
            let fs = fs_lock
                .as_ref()
                .expect("tried to use remote filesystem api when not initialized");

            f(&fs).await
        });
    }
}
