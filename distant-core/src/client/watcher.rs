use crate::{
    client::{SessionChannel, SessionChannelExt, SessionChannelExtError},
    data::{Change, Request, RequestData, ResponseData},
    net::TransportError,
};
use derive_more::{Display, Error};
use std::path::{Path, PathBuf};
use tokio::{sync::mpsc, task::JoinHandle};

#[derive(Debug, Display, Error)]
pub enum WatcherError {
    /// When the communication over the wire has issues
    TransportError(TransportError),

    /// When attempting to unwatch fails
    UnwatchError(SessionChannelExtError),
}

/// Represents a watcher of some path on a remote machine
pub struct Watcher {
    tenant: String,
    channel: SessionChannel,
    path: PathBuf,
    task: JoinHandle<()>,
    rx: mpsc::Receiver<Change>,
}

impl Watcher {
    /// Creates a watcher for some remote path
    pub async fn watch(
        tenant: impl Into<String>,
        mut channel: SessionChannel,
        path: impl Into<PathBuf>,
        recursive: bool,
    ) -> Result<Self, WatcherError> {
        let tenant = tenant.into();
        let path = path.into();

        // Submit our run request and get back a mailbox for responses
        let mut mailbox = channel
            .mail(Request::new(
                tenant.as_str(),
                vec![RequestData::Watch {
                    path: path.to_path_buf(),
                    recursive,
                }],
            ))
            .await
            .map_err(WatcherError::TransportError)?;

        // Spawn a task that continues to look for change events, discarding anything
        // else that it gets
        let (tx, rx) = mpsc::channel(1);
        let task = tokio::spawn(async move {
            while let Some(res) = mailbox.next().await {
                for data in res.payload {
                    match data {
                        ResponseData::Changed { changes } => {
                            for change in changes {
                                // If we can't queue up a change anymore, we've
                                // been closed and therefore want to quit
                                if tx.send(change).await.is_err() {
                                    break;
                                }
                            }
                        }
                        _ => continue,
                    }
                }
            }
        });

        Ok(Self {
            tenant,
            path,
            channel,
            task,
            rx,
        })
    }

    /// Returns a reference to the path this watcher is monitoring
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Returns the next change detected by the watcher, or none if the watcher has concluded
    pub async fn next(&mut self) -> Option<Change> {
        self.rx.recv().await
    }

    /// Unwatches the path being watched, closing out the watcher
    pub async fn unwatch(&mut self) -> Result<(), WatcherError> {
        let result = self
            .channel
            .unwatch(self.tenant.to_string(), self.path.to_path_buf())
            .await
            .map_err(WatcherError::UnwatchError);

        match result {
            Ok(_) => {
                // Kill our task that processes inbound changes if we
                // have successfully unwatched the path
                self.task.abort();

                Ok(())
            }
            Err(x) => Err(x),
        }
    }
}

