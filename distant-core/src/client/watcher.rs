use crate::{
    client::{DistantChannel, DistantChannelError, DistantChannelExt},
    constants::CLIENT_WATCHER_CAPACITY,
    data::{Change, ChangeKindSet, DistantRequestData, DistantResponseData, Error as DistantError},
};
use derive_more::{Display, Error, From};
use distant_net::Request;
use log::*;
use std::{
    fmt, io,
    path::{Path, PathBuf},
};
use tokio::{sync::mpsc, task::JoinHandle};

#[derive(Debug, Display, Error)]
pub enum WatchError {
    /// When no confirmation of watch is received
    MissingConfirmation,

    /// A server-side error occurred when attempting to watch
    ServerError(DistantError),

    /// When the communication over the wire has issues
    IoError(io::Error),

    /// When a queued change is dropped because the response channel closed early
    QueuedChangeDropped,

    /// Some unexpected response was received when attempting to watch
    #[display(fmt = "Unexpected response: {:?}", _0)]
    UnexpectedResponse(#[error(not(source))] DistantResponseData),
}

#[derive(Debug, Display, From, Error)]
pub struct UnwatchError(DistantChannelError);

/// Represents a watcher of some path on a remote machine
pub struct Watcher {
    channel: DistantChannel,
    path: PathBuf,
    task: JoinHandle<()>,
    rx: mpsc::Receiver<Change>,
    active: bool,
}

impl fmt::Debug for Watcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Watcher").field("path", &self.path).finish()
    }
}

impl Watcher {
    /// Creates a watcher for some remote path
    pub async fn watch(
        mut channel: DistantChannel,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
        except: impl Into<ChangeKindSet>,
    ) -> Result<Self, WatchError> {
        let path = path.into();
        let only = only.into();
        let except = except.into();
        trace!(
            "Watching {:?} (recursive = {}){}{}",
            path,
            recursive,
            if only.is_empty() {
                String::new()
            } else {
                format!(" (only = {})", only)
            },
            if except.is_empty() {
                String::new()
            } else {
                format!(" (except = {})", except)
            },
        );

        // Submit our run request and get back a mailbox for responses
        let mut mailbox = channel
            .mail(Request::new(vec![DistantRequestData::Watch {
                path: path.to_path_buf(),
                recursive,
                only: only.into_vec(),
                except: except.into_vec(),
            }]))
            .await
            .map_err(WatchError::TransportError)?;

        let (tx, rx) = mpsc::channel(CLIENT_WATCHER_CAPACITY);

        // Wait to get the confirmation of watch as either ok or error
        let mut queue: Vec<Change> = Vec::new();
        let mut confirmed = false;
        while let Some(res) = mailbox.next().await {
            for data in res.payload {
                match data {
                    DistantResponseData::Changed(change) => queue.push(change),
                    DistantResponseData::Ok => {
                        confirmed = true;
                    }
                    DistantResponseData::Error(x) => return Err(WatchError::ServerError(x)),
                    x => return Err(WatchError::UnexpectedResponse(x)),
                }
            }

            // Exit if we got the confirmation
            // NOTE: Doing this later because we want to make sure the entire payload is processed
            //       first before exiting the loop
            if confirmed {
                break;
            }
        }

        // Send out any of our queued changes that we got prior to the acknowledgement
        trace!("Forwarding {} queued changes for {:?}", queue.len(), path);
        for change in queue {
            if tx.send(change).await.is_err() {
                return Err(WatchError::QueuedChangeDropped);
            }
        }

        // If we never received an acknowledgement of watch before the mailbox closed,
        // fail with a missing confirmation error
        if !confirmed {
            return Err(WatchError::MissingConfirmation);
        }

        // Spawn a task that continues to look for change events, discarding anything
        // else that it gets
        let task = tokio::spawn({
            let path = path.clone();
            async move {
                while let Some(res) = mailbox.next().await {
                    for data in res.payload {
                        match data {
                            DistantResponseData::Changed(change) => {
                                // If we can't queue up a change anymore, we've
                                // been closed and therefore want to quit
                                if tx.is_closed() {
                                    break;
                                }

                                // Otherwise, send over the change
                                if let Err(x) = tx.send(change).await {
                                    error!(
                                        "Watcher for {:?} failed to send change {:?}",
                                        path, x.0
                                    );
                                    break;
                                }
                            }
                            _ => continue,
                        }
                    }
                }
            }
        });

        Ok(Self {
            path,
            channel,
            task,
            rx,
            active: true,
        })
    }

    /// Returns a reference to the path this watcher is monitoring
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Returns true if the watcher is still actively watching for changes
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Returns the next change detected by the watcher, or none if the watcher has concluded
    pub async fn next(&mut self) -> Option<Change> {
        self.rx.recv().await
    }

    /// Unwatches the path being watched, closing out the watcher
    pub async fn unwatch(&mut self) -> Result<(), UnwatchError> {
        trace!("Unwatching {:?}", self.path);
        let result = self
            .channel
            .unwatch(self.path.to_path_buf())
            .await
            .map_err(UnwatchError::from);

        match result {
            Ok(_) => {
                // Kill our task that processes inbound changes if we
                // have successfully unwatched the path
                self.task.abort();
                self.active = false;

                Ok(())
            }
            Err(x) => Err(x),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::ChangeKind;
    use crate::DistantSession;
    use distant_net::{Client, FramedTransport, InmemoryTransport, PlainCodec, Response};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn make_session() -> (
        FramedTransport<InmemoryTransport, PlainCodec>,
        DistantSession,
    ) {
        let (t1, t2) = FramedTransport::make_test_pair();
        (t1, Client::new(t2).unwrap())
    }

    #[tokio::test]
    async fn watcher_should_have_path_reflect_watched_path() {
        let (mut transport, session) = make_session();
        let test_path = Path::new("/some/test/path");

        // Create a task for watcher as we need to handle the request and a response
        // in a separate async block
        let watch_task = tokio::spawn(async move {
            Watcher::watch(
                session.clone_channel(),
                test_path,
                true,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        transport
            .send(Response::new(req.id, vec![DistantResponseData::Ok]))
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
                session.clone_channel(),
                test_path,
                true,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        transport
            .send(Response::new(req.id, vec![DistantResponseData::Ok]))
            .await
            .unwrap();

        // Get the watcher
        let mut watcher = watch_task.await.unwrap().unwrap();

        // Send some changes related to the file
        transport
            .send(Response::new(
                req.id,
                vec![
                    DistantResponseData::Changed(Change {
                        kind: ChangeKind::Access,
                        paths: vec![test_path.to_path_buf()],
                    }),
                    DistantResponseData::Changed(Change {
                        kind: ChangeKind::Content,
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
                kind: ChangeKind::Content,
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
                session.clone_channel(),
                test_path,
                true,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        transport
            .send(Response::new(req.id, vec![DistantResponseData::Ok]))
            .await
            .unwrap();

        // Get the watcher
        let mut watcher = watch_task.await.unwrap().unwrap();

        // Send a change from the appropriate origin
        transport
            .send(Response::new(
                req.id,
                vec![DistantResponseData::Changed(Change {
                    kind: ChangeKind::Access,
                    paths: vec![test_path.to_path_buf()],
                })],
            ))
            .await
            .unwrap();

        // Send a change from a different origin
        transport
            .send(Response::new(
                req.id + 1,
                vec![DistantResponseData::Changed(Change {
                    kind: ChangeKind::Content,
                    paths: vec![test_path.to_path_buf()],
                })],
            ))
            .await
            .unwrap();

        // Send a change from the appropriate origin
        transport
            .send(Response::new(
                req.id,
                vec![DistantResponseData::Changed(Change {
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
                session.clone_channel(),
                test_path,
                true,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back an acknowledgement that a watcher was created
        transport
            .send(Response::new(req.id, vec![DistantResponseData::Ok]))
            .await
            .unwrap();

        // Send some changes from the appropriate origin
        transport
            .send(Response::new(
                req.id,
                vec![
                    DistantResponseData::Changed(Change {
                        kind: ChangeKind::Access,
                        paths: vec![test_path.to_path_buf()],
                    }),
                    DistantResponseData::Changed(Change {
                        kind: ChangeKind::Content,
                        paths: vec![test_path.to_path_buf()],
                    }),
                    DistantResponseData::Changed(Change {
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
            .send(Response::new(req.id, vec![DistantResponseData::Ok]))
            .await
            .unwrap();

        // Wait for the unwatch to complete
        let _ = unwatch_task.await.unwrap().unwrap();

        transport
            .send(Response::new(
                req.id,
                vec![DistantResponseData::Changed(Change {
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
                kind: ChangeKind::Content,
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
