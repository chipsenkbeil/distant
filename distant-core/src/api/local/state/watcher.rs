use crate::{constants::SERVER_WATCHER_CAPACITY, data::ChangeKind, ConnectionId};
use log::*;
use notify::{
    Config as WatcherConfig, Error as WatcherError, ErrorKind as WatcherErrorKind,
    Event as WatcherEvent, PollWatcher, RecursiveMode, Watcher,
};
use std::{
    collections::HashMap,
    io,
    ops::Deref,
    path::{Path, PathBuf},
};
use tokio::{
    sync::{
        mpsc::{self, error::TrySendError},
        oneshot,
    },
    task::JoinHandle,
};

mod path;
pub use path::*;

/// Holds information related to watched paths on the server
pub struct WatcherState {
    channel: WatcherChannel,
    task: JoinHandle<()>,
}

impl Drop for WatcherState {
    /// Aborts the task that handles watcher path operations and management
    fn drop(&mut self) {
        self.abort();
    }
}

impl WatcherState {
    /// Will create a watcher and initialize watched paths to be empty
    pub fn initialize() -> io::Result<Self> {
        // NOTE: Cannot be something small like 1 as this seems to cause a deadlock sometimes
        //       with a large volume of watch requests
        let (tx, rx) = mpsc::channel(SERVER_WATCHER_CAPACITY);

        macro_rules! configure_and_spawn {
            ($watcher:ident) => {{
                // Attempt to configure watcher, but don't fail if these configurations fail
                match $watcher.configure(WatcherConfig::PreciseEvents(true)) {
                    Ok(true) => debug!("Watcher configured for precise events"),
                    Ok(false) => debug!("Watcher not configured for precise events",),
                    Err(x) => error!("Watcher configuration for precise events failed: {}", x),
                }

                // Attempt to configure watcher, but don't fail if these configurations fail
                match $watcher.configure(WatcherConfig::NoticeEvents(true)) {
                    Ok(true) => debug!("Watcher configured for notice events"),
                    Ok(false) => debug!("Watcher not configured for notice events",),
                    Err(x) => error!("Watcher configuration for notice events failed: {}", x),
                }

                Ok(Self {
                    channel: WatcherChannel { tx },
                    task: tokio::spawn(watcher_task($watcher, rx)),
                })
            }};
        }

        macro_rules! event_handler {
            ($tx:ident) => {
                move |res| match $tx.try_send(match res {
                    Ok(x) => InnerWatcherMsg::Event { ev: x },
                    Err(x) => InnerWatcherMsg::Error { err: x },
                }) {
                    Ok(_) => (),
                    Err(TrySendError::Full(_)) => {
                        warn!(
                            "Reached watcher capacity of {}! Dropping watcher event!",
                            SERVER_WATCHER_CAPACITY,
                        );
                    }
                    Err(TrySendError::Closed(_)) => {
                        warn!("Skipping watch event because watcher channel closed");
                    }
                }
            };
        }

        let tx = tx.clone();
        let result = {
            let tx = tx.clone();
            notify::recommended_watcher(event_handler!(tx))
        };

        match result {
            Ok(mut watcher) => configure_and_spawn!(watcher),
            Err(x) => match x.kind {
                // notify-rs has a bug on Mac M1 with Docker and Linux, so we detect that error
                // and fall back to the poll watcher if this occurs
                //
                // https://github.com/notify-rs/notify/issues/423
                WatcherErrorKind::Io(x) if x.raw_os_error() == Some(38) => {
                    warn!("Recommended watcher is unsupported! Falling back to polling watcher!");
                    let mut watcher = PollWatcher::new(event_handler!(tx))
                        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
                    configure_and_spawn!(watcher)
                }
                _ => Err(io::Error::new(io::ErrorKind::Other, x)),
            },
        }
    }

    pub fn clone_channel(&self) -> WatcherChannel {
        self.channel.clone()
    }

    /// Aborts the watcher task
    pub fn abort(&self) {
        self.task.abort();
    }
}

impl Deref for WatcherState {
    type Target = WatcherChannel;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

#[derive(Clone)]
pub struct WatcherChannel {
    tx: mpsc::Sender<InnerWatcherMsg>,
}

impl Default for WatcherChannel {
    /// Creates a new channel that is closed by default
    fn default() -> Self {
        let (tx, _) = mpsc::channel(1);
        Self { tx }
    }
}

impl WatcherChannel {
    /// Watch a path for a specific connection denoted by the id within the registered path
    pub async fn watch(&self, registered_path: RegisteredPath) -> io::Result<()> {
        let (cb, rx) = oneshot::channel();
        self.tx
            .send(InnerWatcherMsg::Watch {
                registered_path,
                cb,
            })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Internal watcher task closed"))?;
        rx.await
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Response to watch dropped"))?
    }

    /// Unwatch a path for a specific connection denoted by the id
    pub async fn unwatch(&self, id: ConnectionId, path: impl AsRef<Path>) -> io::Result<()> {
        let (cb, rx) = oneshot::channel();
        let path = tokio::fs::canonicalize(path.as_ref())
            .await
            .unwrap_or_else(|_| path.as_ref().to_path_buf());
        self.tx
            .send(InnerWatcherMsg::Unwatch { id, path, cb })
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Internal watcher task closed"))?;
        rx.await
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Response to unwatch dropped"))?
    }
}

/// Internal message to pass to our task below to perform some action
enum InnerWatcherMsg {
    Watch {
        registered_path: RegisteredPath,
        cb: oneshot::Sender<io::Result<()>>,
    },
    Unwatch {
        id: ConnectionId,
        path: PathBuf,
        cb: oneshot::Sender<io::Result<()>>,
    },
    Event {
        ev: WatcherEvent,
    },
    Error {
        err: WatcherError,
    },
}

async fn watcher_task(mut watcher: impl Watcher, mut rx: mpsc::Receiver<InnerWatcherMsg>) {
    // TODO: Optimize this in some way to be more performant than
    //       checking every path whenever an event comes in
    let mut registered_paths: Vec<RegisteredPath> = Vec::new();
    let mut path_cnt: HashMap<PathBuf, usize> = HashMap::new();

    while let Some(msg) = rx.recv().await {
        match msg {
            InnerWatcherMsg::Watch {
                registered_path,
                cb,
            } => {
                // Check if we are tracking the path across any connection
                if let Some(cnt) = path_cnt.get_mut(registered_path.path()) {
                    // Increment the count of times we are watching that path
                    *cnt += 1;

                    // Store the registered path in our collection without worry
                    // since we are already watching a path that impacts this one
                    registered_paths.push(registered_path);

                    // Send an okay because we always succeed in this case
                    let _ = cb.send(Ok(()));
                } else {
                    let res = watcher
                        .watch(
                            registered_path.path(),
                            if registered_path.is_recursive() {
                                RecursiveMode::Recursive
                            } else {
                                RecursiveMode::NonRecursive
                            },
                        )
                        .map_err(|x| io::Error::new(io::ErrorKind::Other, x));

                    // If we succeeded, store our registered path and set the tracking cnt to 1
                    if res.is_ok() {
                        path_cnt.insert(registered_path.path().to_path_buf(), 1);
                        registered_paths.push(registered_path);
                    }

                    // Send the result of the watch, but don't worry if the channel was closed
                    let _ = cb.send(res);
                }
            }
            InnerWatcherMsg::Unwatch { id, path, cb } => {
                // Check if we are tracking the path across any connection
                if let Some(cnt) = path_cnt.get(path.as_path()) {
                    // Cycle through and remove all paths that match the given id and path,
                    // capturing how many paths we removed
                    let removed_cnt = {
                        let old_len = registered_paths.len();
                        registered_paths
                            .retain(|p| p.id() != id || (p.path() != path && p.raw_path() != path));
                        let new_len = registered_paths.len();
                        old_len - new_len
                    };

                    // 1. If we are now at zero cnt for our path, we want to actually unwatch the
                    //    path with our watcher
                    // 2. If we removed nothing from our path list, we want to return an error
                    // 3. Otherwise, we return okay because we succeeded
                    if *cnt <= removed_cnt {
                        let _ = cb.send(
                            watcher
                                .unwatch(&path)
                                .map_err(|x| io::Error::new(io::ErrorKind::Other, x)),
                        );
                    } else if removed_cnt == 0 {
                        // Send a failure as there was nothing to unwatch for this connection
                        let _ = cb.send(Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!("{:?} is not being watched", path),
                        )));
                    } else {
                        // Send a success as we removed some paths
                        let _ = cb.send(Ok(()));
                    }
                } else {
                    // Send a failure as there was nothing to unwatch
                    let _ = cb.send(Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("{:?} is not being watched", path),
                    )));
                }
            }
            InnerWatcherMsg::Event { ev } => {
                let kind = ChangeKind::from(ev.kind);

                for registered_path in registered_paths.iter() {
                    match registered_path.filter_and_send(kind, &ev.paths).await {
                        Ok(_) => (),
                        Err(x) => error!(
                            "[Conn {}] Failed to forward changes to paths: {}",
                            registered_path.id(),
                            x
                        ),
                    }
                }
            }
            InnerWatcherMsg::Error { err } => {
                let msg = err.to_string();
                error!("Watcher encountered an error {} for {:?}", msg, err.paths);

                for registered_path in registered_paths.iter() {
                    match registered_path
                        .filter_and_send_error(&msg, &err.paths, !err.paths.is_empty())
                        .await
                    {
                        Ok(_) => (),
                        Err(x) => error!(
                            "[Conn {}] Failed to forward changes to paths: {}",
                            registered_path.id(),
                            x
                        ),
                    }
                }
            }
        }
    }
}
