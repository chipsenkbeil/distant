use crate::{
    client::{SessionChannel, SessionChannelExt, SessionChannelExtError},
    constants::CLIENT_WATCHER_CAPACITY,
    data::{Change, ChangeKindSet, Request, RequestData, ResponseData},
    net::TransportError,
};
use derive_more::{Display, Error};
use std::{
    fmt,
    path::{Path, PathBuf},
};
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

impl fmt::Debug for Watcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Watcher")
            .field("tenant", &self.tenant)
            .field("path", &self.path)
            .finish()
    }
}

impl Watcher {
    /// Creates a watcher for some remote path
    pub async fn watch(
        tenant: impl Into<String>,
        mut channel: SessionChannel,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
    ) -> Result<Self, WatcherError> {
        let tenant = tenant.into();
        let path = path.into();
        let only = only.into();

        // Submit our run request and get back a mailbox for responses
        let mut mailbox = channel
            .mail(Request::new(
                tenant.as_str(),
                vec![RequestData::Watch {
                    path: path.to_path_buf(),
                    recursive,
                    only,
                }],
            ))
            .await
            .map_err(WatcherError::TransportError)?;

        // Spawn a task that continues to look for change events, discarding anything
        // else that it gets
        let (tx, rx) = mpsc::channel(CLIENT_WATCHER_CAPACITY);
        let task = tokio::spawn(async move {
            while let Some(res) = mailbox.next().await {
                for data in res.payload {
                    match data {
                        ResponseData::Changed(change) => {
                            // If we can't queue up a change anymore, we've
                            // been closed and therefore want to quit
                            if tx.send(change).await.is_err() {
                                break;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::Session,
        data::{ChangeKind, Response},
        net::{InmemoryStream, PlainCodec, Transport},
    };
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn make_session() -> (Transport<InmemoryStream, PlainCodec>, Session) {
        let (t1, t2) = Transport::make_pair();
        (t1, Session::initialize(t2).unwrap())
    }

    #[tokio::test]
    async fn watcher_should_have_path_reflect_watched_path() {
        let (mut transport, session) = make_session();
        let test_path = Path::new("/some/test/path");

        // Create a task for watcher as we need to handle the request and a response
        // in a separate async block
        let watch_task = tokio::spawn(async move {
            Watcher::watch(
                String::from("test-tenant"),
                session.clone_channel(),
                test_path,
                true,
                ChangeKindSet::default(),
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        transport
            .send(Response::new("test-tenant", req.id, vec![ResponseData::Ok]))
            .await
            .unwrap();

        // Get the watcher and verify the path
        let watcher = watch_task.await.unwrap().unwrap();
        assert_eq!(watcher.path(), test_path);
    }

    #[tokio::test]
    async fn watcher_should_support_getting_next_change() {
        let (mut transport, session) = make_session();
        let test_path = Path::new("/some/test/path");

        // Create a task for watcher as we need to handle the request and a response
        // in a separate async block
        let watch_task = tokio::spawn(async move {
            Watcher::watch(
                String::from("test-tenant"),
                session.clone_channel(),
                test_path,
                true,
                ChangeKindSet::default(),
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        transport
            .send(Response::new("test-tenant", req.id, vec![ResponseData::Ok]))
            .await
            .unwrap();

        // Get the watcher
        let mut watcher = watch_task.await.unwrap().unwrap();

        // Send some changes related to the file
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![
                    ResponseData::Changed(Change {
                        kind: ChangeKind::Access,
                        paths: vec![test_path.to_path_buf()],
                    }),
                    ResponseData::Changed(Change {
                        kind: ChangeKind::Modify,
                        paths: vec![test_path.to_path_buf()],
                    }),
                ],
            ))
            .await
            .unwrap();

        // Verify that the watcher gets the changes, one at a time
        let change = watcher.next().await.expect("Watcher closed unexpectedly");
        assert_eq!(
            change,
            Change {
                kind: ChangeKind::Access,
                paths: vec![test_path.to_path_buf()]
            }
        );

        let change = watcher.next().await.expect("Watcher closed unexpectedly");
        assert_eq!(
            change,
            Change {
                kind: ChangeKind::Modify,
                paths: vec![test_path.to_path_buf()]
            }
        );
    }

    #[tokio::test]
    async fn watcher_should_distinguish_change_events_and_only_receive_changes_for_itself() {
        let (mut transport, session) = make_session();
        let test_path = Path::new("/some/test/path");

        // Create a task for watcher as we need to handle the request and a response
        // in a separate async block
        let watch_task = tokio::spawn(async move {
            Watcher::watch(
                String::from("test-tenant"),
                session.clone_channel(),
                test_path,
                true,
                ChangeKindSet::default(),
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        transport
            .send(Response::new("test-tenant", req.id, vec![ResponseData::Ok]))
            .await
            .unwrap();

        // Get the watcher
        let mut watcher = watch_task.await.unwrap().unwrap();

        // Send a change from the appropriate origin
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::Changed(Change {
                    kind: ChangeKind::Access,
                    paths: vec![test_path.to_path_buf()],
                })],
            ))
            .await
            .unwrap();

        // Send a change from a different origin
        transport
            .send(Response::new(
                "test-tenant",
                req.id + 1,
                vec![ResponseData::Changed(Change {
                    kind: ChangeKind::Modify,
                    paths: vec![test_path.to_path_buf()],
                })],
            ))
            .await
            .unwrap();

        // Send a change from the appropriate origin
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::Changed(Change {
                    kind: ChangeKind::Remove,
                    paths: vec![test_path.to_path_buf()],
                })],
            ))
            .await
            .unwrap();

        // Verify that the watcher gets the changes, one at a time
        let change = watcher.next().await.expect("Watcher closed unexpectedly");
        assert_eq!(
            change,
            Change {
                kind: ChangeKind::Access,
                paths: vec![test_path.to_path_buf()]
            }
        );

        let change = watcher.next().await.expect("Watcher closed unexpectedly");
        assert_eq!(
            change,
            Change {
                kind: ChangeKind::Remove,
                paths: vec![test_path.to_path_buf()]
            }
        );
    }

    #[tokio::test]
    async fn watcher_should_stop_receiving_events_if_unwatched() {
        let (mut transport, session) = make_session();
        let test_path = Path::new("/some/test/path");

        // Create a task for watcher as we need to handle the request and a response
        // in a separate async block
        let watch_task = tokio::spawn(async move {
            Watcher::watch(
                String::from("test-tenant"),
                session.clone_channel(),
                test_path,
                true,
                ChangeKindSet::default(),
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        transport
            .send(Response::new("test-tenant", req.id, vec![ResponseData::Ok]))
            .await
            .unwrap();

        // Send some changes from the appropriate origin
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![
                    ResponseData::Changed(Change {
                        kind: ChangeKind::Access,
                        paths: vec![test_path.to_path_buf()],
                    }),
                    ResponseData::Changed(Change {
                        kind: ChangeKind::Modify,
                        paths: vec![test_path.to_path_buf()],
                    }),
                    ResponseData::Changed(Change {
                        kind: ChangeKind::Remove,
                        paths: vec![test_path.to_path_buf()],
                    }),
                ],
            ))
            .await
            .unwrap();

        // Wait a little bit for all changes to be queued
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Create a task for for unwatching as we need to handle the request and a response
        // in a separate async block
        let watcher = Arc::new(Mutex::new(watch_task.await.unwrap().unwrap()));

        // Verify that the watcher gets the first change
        let change = watcher
            .lock()
            .await
            .next()
            .await
            .expect("Watcher closed unexpectedly");
        assert_eq!(
            change,
            Change {
                kind: ChangeKind::Access,
                paths: vec![test_path.to_path_buf()]
            }
        );

        // Unwatch the watcher, verify the request is sent out, and respond with ok
        let watcher_2 = Arc::clone(&watcher);
        let unwatch_task = tokio::spawn(async move { watcher_2.lock().await.unwatch().await });

        let req = transport.receive::<Request>().await.unwrap().unwrap();

        transport
            .send(Response::new("test-tenant", req.id, vec![ResponseData::Ok]))
            .await
            .unwrap();

        // Wait for the unwatch to complete
        let _ = unwatch_task.await.unwrap().unwrap();

        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::Changed(Change {
                    kind: ChangeKind::Unknown,
                    paths: vec![test_path.to_path_buf()],
                })],
            ))
            .await
            .unwrap();

        // Verify that we get any remaining changes that were received before unwatched,
        // but nothing new after that
        assert_eq!(
            watcher.lock().await.next().await,
            Some(Change {
                kind: ChangeKind::Modify,
                paths: vec![test_path.to_path_buf()]
            })
        );
        assert_eq!(
            watcher.lock().await.next().await,
            Some(Change {
                kind: ChangeKind::Remove,
                paths: vec![test_path.to_path_buf()]
            })
        );
        assert_eq!(watcher.lock().await.next().await, None);
    }
}
