use std::collections::HashMap;
use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use distant_core::net::common::ConnectionId;
use distant_core::protocol::{Change, ChangeDetails, ChangeDetailsAttributes, ChangeKind};
use log::*;
use notify::event::{AccessKind, AccessMode, MetadataKind, ModifyKind};
use notify::{
    Config as WatcherConfig, Error as WatcherError, ErrorKind as WatcherErrorKind,
    Event as WatcherEvent, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher,
};
use notify_debouncer_full::{new_debouncer_opt, DebounceEventResult, Debouncer, FileIdMap};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::config::WatchConfig;
use crate::constants::SERVER_WATCHER_CAPACITY;

mod path;
pub use path::*;

/// Builder for a watcher.
#[derive(Default)]
pub struct WatcherBuilder {
    config: WatchConfig,
}

impl WatcherBuilder {
    /// Creates a new builder configured to use the native watcher using default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Swaps the configuration with the provided one.
    pub fn with_config(self, config: WatchConfig) -> Self {
        Self { config }
    }

    /// Will create a watcher and initialize watched paths to be empty
    pub fn initialize(self) -> io::Result<WatcherState> {
        // NOTE: Cannot be something small like 1 as this seems to cause a deadlock sometimes
        //       with a large volume of watch requests
        let (tx, rx) = mpsc::channel(SERVER_WATCHER_CAPACITY);

        let watcher_config = WatcherConfig::default()
            .with_compare_contents(self.config.compare_contents)
            .with_poll_interval(self.config.poll_interval.unwrap_or(Duration::from_secs(30)));

        macro_rules! process_event {
            ($tx:ident, $evt:expr) => {
                match $tx.try_send(match $evt {
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

        macro_rules! new_debouncer {
            ($watcher:ident, $tx:ident) => {{
                new_debouncer_opt::<_, $watcher, FileIdMap>(
                    self.config.debounce_timeout,
                    self.config.debounce_tick_rate,
                    move |result: DebounceEventResult| match result {
                        Ok(events) => {
                            for x in events {
                                process_event!($tx, Ok(x));
                            }
                        }
                        Err(errors) => {
                            for x in errors {
                                process_event!($tx, Err(x));
                            }
                        }
                    },
                    FileIdMap::new(),
                    watcher_config,
                )
            }};
        }

        macro_rules! spawn_task {
            ($debouncer:expr) => {{
                WatcherState {
                    channel: WatcherChannel { tx },
                    task: tokio::spawn(watcher_task($debouncer, rx)),
                }
            }};
        }

        let tx = tx.clone();
        if self.config.native {
            let result = {
                let tx = tx.clone();
                new_debouncer!(RecommendedWatcher, tx)
            };

            match result {
                Ok(debouncer) => Ok(spawn_task!(debouncer)),
                Err(x) => {
                    match x.kind {
                        // notify-rs has a bug on Mac M1 with Docker and Linux, so we detect that error
                        // and fall back to the poll watcher if this occurs
                        //
                        // https://github.com/notify-rs/notify/issues/423
                        WatcherErrorKind::Io(x) if x.raw_os_error() == Some(38) => {
                            warn!("Recommended watcher is unsupported! Falling back to polling watcher!");
                            Ok(spawn_task!(new_debouncer!(PollWatcher, tx)
                                .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?))
                        }
                        _ => Err(io::Error::new(io::ErrorKind::Other, x)),
                    }
                }
            }
        } else {
            Ok(spawn_task!(new_debouncer!(PollWatcher, tx)
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?))
        }
    }
}

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

async fn watcher_task<W>(
    mut debouncer: Debouncer<W, FileIdMap>,
    mut rx: mpsc::Receiver<InnerWatcherMsg>,
) where
    W: Watcher,
{
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
                    let res = debouncer
                        .watcher()
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
                            debouncer
                                .watcher()
                                .unwatch(&path)
                                .map_err(|x| io::Error::new(io::ErrorKind::Other, x)),
                        );
                    } else if removed_cnt == 0 {
                        // Send a failure as there was nothing to unwatch for this connection
                        let _ = cb.send(Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!("{path:?} is not being watched"),
                        )));
                    } else {
                        // Send a success as we removed some paths
                        let _ = cb.send(Ok(()));
                    }
                } else {
                    // Send a failure as there was nothing to unwatch
                    let _ = cb.send(Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("{path:?} is not being watched"),
                    )));
                }
            }
            InnerWatcherMsg::Event { ev } => {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("System time before unix epoch")
                    .as_secs();

                let kind = match ev.kind {
                    EventKind::Access(AccessKind::Read) => ChangeKind::Access,
                    EventKind::Modify(ModifyKind::Metadata(_)) => ChangeKind::Attribute,
                    EventKind::Access(AccessKind::Close(AccessMode::Write)) => {
                        ChangeKind::CloseWrite
                    }
                    EventKind::Access(AccessKind::Close(_)) => ChangeKind::CloseNoWrite,
                    EventKind::Create(_) => ChangeKind::Create,
                    EventKind::Remove(_) => ChangeKind::Delete,
                    EventKind::Modify(ModifyKind::Data(_)) => ChangeKind::Modify,
                    EventKind::Access(AccessKind::Open(_)) => ChangeKind::Open,
                    EventKind::Modify(ModifyKind::Name(_)) => ChangeKind::Rename,
                    _ => ChangeKind::Unknown,
                };

                let attributes = match ev.kind {
                    EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)) => {
                        vec![ChangeDetailsAttributes::Timestamp]
                    }
                    EventKind::Modify(ModifyKind::Metadata(
                        MetadataKind::Ownership | MetadataKind::Permissions,
                    )) => vec![ChangeDetailsAttributes::Permissions],
                    _ => Vec::new(),
                };

                for registered_path in registered_paths.iter() {
                    let change = Change {
                        timestamp,
                        kind,
                        paths: ev.paths.clone(),
                        details: ChangeDetails {
                            attributes: attributes.clone(),
                            extra: ev.info().map(ToString::to_string),
                        },
                    };
                    match registered_path.filter_and_send(change).await {
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
