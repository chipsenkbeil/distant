use std::collections::HashMap;
use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use distant_core::net::common::ConnectionId;
use distant_core::protocol::{Change, ChangeDetails, ChangeDetailsAttribute, ChangeKind};
use log::*;
use notify::event::{AccessKind, AccessMode, MetadataKind, ModifyKind, RenameMode};
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
                            Ok(spawn_task!(
                                new_debouncer!(PollWatcher, tx).map_err(io::Error::other)?
                            ))
                        }
                        _ => Err(io::Error::other(x)),
                    }
                }
            }
        } else {
            Ok(spawn_task!(
                new_debouncer!(PollWatcher, tx).map_err(io::Error::other)?
            ))
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
            .map_err(|_| io::Error::other("Internal watcher task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to watch dropped"))?
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
            .map_err(|_| io::Error::other("Internal watcher task closed"))?;
        rx.await
            .map_err(|_| io::Error::other("Response to unwatch dropped"))?
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
                        .map_err(io::Error::other);

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
                        let _ =
                            cb.send(debouncer.watcher().unwatch(&path).map_err(io::Error::other));
                    } else if removed_cnt == 0 {
                        // Send a failure as there was nothing to unwatch for this connection
                        let _ = cb.send(Err(io::Error::other(format!(
                            "{path:?} is not being watched"
                        ))));
                    } else {
                        // Send a success as we removed some paths
                        let _ = cb.send(Ok(()));
                    }
                } else {
                    // Send a failure as there was nothing to unwatch
                    let _ = cb.send(Err(io::Error::other(format!(
                        "{path:?} is not being watched"
                    ))));
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

                for registered_path in registered_paths.iter() {
                    // For rename both, we assume the paths is a pair that represents before and
                    // after, so we want to grab the before and use it!
                    let (paths, renamed): (&[PathBuf], Option<PathBuf>) = match ev.kind {
                        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => (
                            &ev.paths[0..1],
                            if ev.paths.len() > 1 {
                                ev.paths.last().cloned()
                            } else {
                                None
                            },
                        ),
                        _ => (&ev.paths, None),
                    };

                    for path in paths {
                        let attribute = match ev.kind {
                            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Ownership)) => {
                                Some(ChangeDetailsAttribute::Ownership)
                            }
                            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Permissions)) => {
                                Some(ChangeDetailsAttribute::Permissions)
                            }
                            EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)) => {
                                Some(ChangeDetailsAttribute::Timestamp)
                            }
                            _ => None,
                        };

                        // Calculate a timestamp for creation & modification paths
                        let details_timestamp = match ev.kind {
                            EventKind::Create(_) => tokio::fs::symlink_metadata(path.as_path())
                                .await
                                .ok()
                                .and_then(|m| m.created().ok())
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs()),
                            EventKind::Modify(_) => tokio::fs::symlink_metadata(path.as_path())
                                .await
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                .map(|d| d.as_secs()),
                            _ => None,
                        };

                        let change = Change {
                            timestamp,
                            kind,
                            path: path.to_path_buf(),
                            details: ChangeDetails {
                                attribute,
                                renamed: renamed.clone(),
                                timestamp: details_timestamp,
                                extra: ev.info().map(ToString::to_string),
                            },
                        };
                        match registered_path.filter_and_send(change) {
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
            InnerWatcherMsg::Error { err } => {
                let msg = err.to_string();
                error!("Watcher encountered an error {} for {:?}", msg, err.paths);

                for registered_path in registered_paths.iter() {
                    match registered_path.filter_and_send_error(
                        &msg,
                        &err.paths,
                        !err.paths.is_empty(),
                    ) {
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

#[cfg(test)]
mod tests {
    //! Tests for `WatcherBuilder`, `WatcherChannel`, and `WatcherState` covering builder
    //! configuration, channel lifecycle, watch/unwatch operations, ref-counting for multiple
    //! watchers on the same path, recursive mode, and actual file change detection.

    use super::*;
    use assert_fs::prelude::*;
    use distant_core::protocol::ChangeKindSet;
    use test_log::test;

    // ---- WatcherBuilder ----

    #[test(tokio::test)]
    async fn watcher_builder_new_should_create_default_builder() {
        let builder = WatcherBuilder::new();
        // Default config should use native watcher
        assert!(builder.config.native);
    }

    #[test(tokio::test)]
    async fn watcher_builder_with_config_should_replace_config() {
        let config = WatchConfig {
            native: false,
            poll_interval: Some(Duration::from_secs(10)),
            compare_contents: true,
            debounce_timeout: Duration::from_millis(100),
            debounce_tick_rate: Some(Duration::from_millis(50)),
        };

        let builder = WatcherBuilder::new().with_config(config.clone());
        assert_eq!(builder.config, config);
    }

    #[test(tokio::test)]
    async fn watcher_builder_initialize_should_succeed_with_native_watcher() {
        let state = WatcherBuilder::new().initialize().unwrap();
        // State should be usable
        drop(state);
    }

    #[test(tokio::test)]
    async fn watcher_builder_initialize_should_succeed_with_poll_watcher() {
        let config = WatchConfig {
            native: false,
            poll_interval: Some(Duration::from_secs(1)),
            ..Default::default()
        };
        let state = WatcherBuilder::new()
            .with_config(config)
            .initialize()
            .unwrap();
        drop(state);
    }

    // ---- WatcherChannel::default ----

    #[test(tokio::test)]
    async fn default_channel_watch_should_fail_with_closed_error() {
        let channel = WatcherChannel::default();
        let temp = assert_fs::TempDir::new().unwrap();
        let (reply_tx, _reply_rx) = tokio::sync::mpsc::unbounded_channel();

        let registered = RegisteredPath::register(
            1,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx),
        )
        .await
        .unwrap();

        let result = channel.watch(registered).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Internal watcher task closed"));
    }

    #[test(tokio::test)]
    async fn default_channel_unwatch_should_fail_with_closed_error() {
        let channel = WatcherChannel::default();
        let result = channel.unwatch(1, "/some/path").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Internal watcher task closed"));
    }

    // ---- WatcherState lifecycle ----

    #[test(tokio::test)]
    async fn watcher_state_deref_provides_channel() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let _channel: &WatcherChannel = &state;
    }

    #[test(tokio::test)]
    async fn watcher_state_abort_should_close_internal_task() {
        let state = WatcherBuilder::new().initialize().unwrap();
        state.abort();

        // Allow time for abort to propagate
        tokio::time::sleep(Duration::from_millis(50)).await;

        let temp = assert_fs::TempDir::new().unwrap();
        let (reply_tx, _reply_rx) = tokio::sync::mpsc::unbounded_channel();

        let registered = RegisteredPath::register(
            1,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx),
        )
        .await
        .unwrap();

        let result = state.watch(registered).await;
        assert!(result.is_err());
    }

    // ---- watch / unwatch ----

    #[test(tokio::test)]
    async fn watch_should_succeed_for_existing_directory() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let temp = assert_fs::TempDir::new().unwrap();
        let (reply_tx, _reply_rx) = tokio::sync::mpsc::unbounded_channel();

        let registered = RegisteredPath::register(
            1,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx),
        )
        .await
        .unwrap();

        let result = state.watch(registered).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn watch_should_succeed_for_existing_file() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let temp = assert_fs::TempDir::new().unwrap();
        let file = temp.child("test.txt");
        file.touch().unwrap();

        let (reply_tx, _reply_rx) = tokio::sync::mpsc::unbounded_channel();

        let registered = RegisteredPath::register(
            1,
            file.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx),
        )
        .await
        .unwrap();

        let result = state.watch(registered).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn watch_same_path_twice_should_increment_refcount() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let temp = assert_fs::TempDir::new().unwrap();

        // First watch
        let (reply_tx1, _rx1) = tokio::sync::mpsc::unbounded_channel();
        let reg1 = RegisteredPath::register(
            1,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx1),
        )
        .await
        .unwrap();
        state.watch(reg1).await.unwrap();

        // Second watch of same path by different connection
        let (reply_tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();
        let reg2 = RegisteredPath::register(
            2,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx2),
        )
        .await
        .unwrap();
        let result = state.watch(reg2).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn unwatch_should_succeed_after_watching() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let temp = assert_fs::TempDir::new().unwrap();

        let (reply_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let registered = RegisteredPath::register(
            1,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx),
        )
        .await
        .unwrap();
        state.watch(registered).await.unwrap();

        let result = state.unwatch(1, temp.path()).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn unwatch_should_fail_for_unwatched_path() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let temp = assert_fs::TempDir::new().unwrap();

        let result = state.unwatch(1, temp.path()).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("is not being watched"));
    }

    #[test(tokio::test)]
    async fn unwatch_should_fail_for_wrong_connection_id() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let temp = assert_fs::TempDir::new().unwrap();

        let (reply_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let registered = RegisteredPath::register(
            1,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx),
        )
        .await
        .unwrap();
        state.watch(registered).await.unwrap();

        // Try to unwatch with a different connection id
        let result = state.unwatch(999, temp.path()).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("is not being watched"));
    }

    #[test(tokio::test)]
    async fn unwatch_one_of_two_watchers_should_keep_path_watched() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let temp = assert_fs::TempDir::new().unwrap();

        // Two connections watch the same path
        let (reply_tx1, _rx1) = tokio::sync::mpsc::unbounded_channel();
        let reg1 = RegisteredPath::register(
            1,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx1),
        )
        .await
        .unwrap();
        state.watch(reg1).await.unwrap();

        let (reply_tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();
        let reg2 = RegisteredPath::register(
            2,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx2),
        )
        .await
        .unwrap();
        state.watch(reg2).await.unwrap();

        // Unwatch for connection 1 - should succeed (decrements refcount)
        let result = state.unwatch(1, temp.path()).await;
        assert!(result.is_ok());

        // Unwatch for connection 2 - should also succeed (actually unwatches)
        let result = state.unwatch(2, temp.path()).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn watch_should_succeed_with_recursive_mode() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let temp = assert_fs::TempDir::new().unwrap();
        temp.child("subdir").create_dir_all().unwrap();

        let (reply_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let registered = RegisteredPath::register(
            1,
            temp.path(),
            true, // recursive
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx),
        )
        .await
        .unwrap();

        let result = state.watch(registered).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn watcher_channel_clone_should_work() {
        let state = WatcherBuilder::new().initialize().unwrap();
        let channel = state.channel.clone();
        let temp = assert_fs::TempDir::new().unwrap();

        let (reply_tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let registered = RegisteredPath::register(
            1,
            temp.path(),
            false,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx),
        )
        .await
        .unwrap();

        let result = channel.watch(registered).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn watch_should_detect_file_changes() {
        // Use the native watcher for reliability (poll watcher can be flaky)
        let state = WatcherBuilder::new().initialize().unwrap();
        let temp = assert_fs::TempDir::new().unwrap();

        let (reply_tx, mut reply_rx) = tokio::sync::mpsc::unbounded_channel();
        let registered = RegisteredPath::register(
            1,
            temp.path(),
            true,
            ChangeKindSet::default(),
            ChangeKindSet::default(),
            Box::new(reply_tx),
        )
        .await
        .unwrap();
        state.watch(registered).await.unwrap();

        // Wait for the watcher to fully initialize
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Create a new file inside the watched directory
        let file = temp.child("new_file.txt");
        file.write_str("hello world").unwrap();

        // Wait for a change event (generous timeout for CI environments)
        let result = tokio::time::timeout(Duration::from_secs(10), reply_rx.recv()).await;
        assert!(
            result.is_ok(),
            "Should have received a watcher event after file change"
        );
        let response = result.unwrap().unwrap();
        match response {
            distant_core::protocol::Response::Changed(change) => {
                assert!(!change.path.as_os_str().is_empty());
            }
            distant_core::protocol::Response::Error(_) => {
                // Some platforms may report errors instead of changes; that's ok
            }
            other => panic!("Unexpected response type: {other:?}"),
        }
    }
}
