use crate::{
    constants::SERVER_WATCHER_CAPACITY,
    data::{Change, ChangeKind, ChangeKindSet, DistantResponseData},
};
use distant_net::QueuedServerReply;
use log::*;
use notify::{
    Config as WatcherConfig, Error as WatcherError, Event as WatcherEvent, RecommendedWatcher,
    RecursiveMode, Watcher,
};
use std::{
    collections::{hash_map::Entry, HashMap},
    hash::{Hash, Hasher},
    io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};
use tokio::sync::{
    mpsc::{self, error::TrySendError},
    Mutex,
};

pub struct WatcherState {
    // NOTE: I think the design of the watcher will only spawn a thread once
    //       watching a path starts, and each new watch will restart the
    //       thread; so, we can create the watcher when the state is
    //       created and not worry about causing unexpected threads
    watcher: RecommendedWatcher,

    /// Contains a collection of connection id -> paths being watched
    paths: Arc<Mutex<HashMap<usize, Vec<WatcherPath>>>>,
}

impl WatcherState {
    pub async fn watch(
        &mut self,
        connection_id: usize,
        path: impl AsRef<Path>,
        recursive: bool,
        only: ChangeKindSet,
        except: ChangeKindSet,
        reply: QueuedServerReply<DistantResponseData>,
    ) -> io::Result<()> {
        let wp = WatcherPath::new(path.as_ref(), recursive, only, except, reply)?;
        self.watcher.watch(
            path.as_ref(),
            if recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            },
        )?;

        match self.paths.lock().await.entry(connection_id) {
            Entry::Occupied(x) => x.get_mut().push(wp),
            Entry::Vacant(x) => x.insert(vec![wp]),
        };

        Ok(())
    }

    pub async fn unwatch(
        &mut self,
        connection_id: usize,
        path: impl AsRef<Path>,
    ) -> io::Result<()> {
        self.watcher.unwatch(path.as_ref()).map_err()?;

        // Remove the path from our list associated with the connection
        if let Some(paths) = self.paths.lock().await.get_mut(&connection_id) {
            let path = path.as_ref();
            paths.retain(|wp| wp.path == path || wp.raw_path == path);
        }

        Ok(())
    }

    /// Will create a watcher and initialize watched paths to be empty
    pub fn initialize() -> io::Result<Self> {
        // NOTE: Cannot be something small like 1 as this seems to cause a deadlock sometimes
        //       with a large volume of watch requests
        let (tx, mut rx) = mpsc::channel(SERVER_WATCHER_CAPACITY);

        let mut watcher = notify::recommended_watcher(move |res| match tx.try_send(res) {
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
        })
        .map_err(io::Error::from)?;

        // Attempt to configure watcher, but don't fail if these configurations fail
        match watcher.configure(WatcherConfig::PreciseEvents(true)) {
            Ok(true) => debug!("Watcher configured for precise events"),
            Ok(false) => debug!("Watcher not configured for precise events",),
            Err(x) => error!("Watcher configuration for precise events failed: {}", x),
        }

        // Attempt to configure watcher, but don't fail if these configurations fail
        match watcher.configure(WatcherConfig::NoticeEvents(true)) {
            Ok(true) => debug!("Watcher configured for notice events"),
            Ok(false) => debug!("Watcher not configured for notice events",),
            Err(x) => error!("Watcher configuration for notice events failed: {}", x),
        }

        let paths = Arc::new(Mutex::new(HashMap::new()));
        Self::spawn_watcher_channel(rx, Arc::downgrade(&paths));

        Ok(Self { watcher, paths })
    }

    fn spawn_watcher_channel(
        mut rx: mpsc::Receiver<Result<WatcherEvent, WatcherError>>,
        paths: Weak<Mutex<Vec<WatcherPath>>>,
    ) {
        tokio::spawn(async move {
            while let Some(res) = rx.recv().await {
                let is_ok = match res {
                    Ok(mut x) => {
                        let ev_paths: Vec<_> = x.paths.drain(..).collect();
                        let kind = ChangeKind::from(x.kind);

                        fn make_res_data(
                            kind: ChangeKind,
                            paths: &[&PathBuf],
                        ) -> DistantResponseData {
                            DistantResponseData::Changed(Change {
                                kind,
                                paths: paths.iter().map(|p| p.to_path_buf()).collect(),
                            })
                        }

                        let results = Self::find_matches(&paths, &ev_paths).await;
                        let mut is_ok = true;

                        for (paths, wp) in results {
                            // Skip sending this change if we are not watching it
                            if (!wp.only.is_empty() && !wp.only.contains(&kind))
                                || (!wp.except.is_empty() && wp.except.contains(&kind))
                            {
                                trace!("Skipping change '{}' for {:?}", kind, paths);
                                continue;
                            }

                            if let Err(x) = wp.reply.send(make_res_data(kind, &paths)).await {
                                error!("Failed to report on changes to paths: {:?}", paths);
                                is_ok = false;
                                break;
                            }
                        }
                        is_ok
                    }
                    Err(mut x) => {
                        let ev_paths: Vec<_> = x.paths.drain(..).collect();
                        let msg = x.to_string();

                        error!("Watcher encountered an error {} for {:?}", msg, ev_paths);

                        fn make_res_data(msg: &str, paths: &[&PathBuf]) -> DistantResponseData {
                            if paths.is_empty() {
                                DistantResponseData::Error(msg.into())
                            } else {
                                DistantResponseData::Error(
                                    format!("{} about {:?}", msg, paths).into(),
                                )
                            }
                        }

                        let mut is_ok = true;

                        // If we have no paths for the errors, then we send the error to everyone
                        if ev_paths.is_empty() {
                            if let Some(paths) = Weak::upgrade(&paths) {
                                trace!("Relaying error to all watching connections");
                                for (paths, wp) in paths.lock().await.values_mut() {
                                    if let Err(x) = wp.reply.send(make_res_data(&msg, &[])).await {
                                        error!("Failed to report on changes to paths: {:?}", paths);
                                        is_ok = false;
                                        break;
                                    }
                                }
                            }
                        // Otherwise, figure out the relevant watchers from our paths and
                        // send the error to them
                        } else {
                            let results = Self::find_matches(&paths, &ev_paths).await;

                            trace!("Relaying error to {} connections", results.len());
                            for (paths, wp) in results {
                                if let Err(x) = wp.reply.send(make_res_data(&msg, &paths)).await {
                                    error!("Failed to report on changes to paths: {:?}", paths);
                                    is_ok = false;
                                    break;
                                }
                            }
                        }

                        is_ok
                    }
                };

                if !is_ok {
                    error!("Watcher channel closed");
                    break;
                }
            }
        });
    }

    /// Given a collection of paths, finds all of the paths that map to a given reply
    /// and returns a list of paths -> sender
    async fn find_matches<'a>(
        watcher_paths: &Weak<Mutex<Vec<WatcherPath>>>,
        paths: &'a [PathBuf],
    ) -> Vec<(Vec<&'a PathBuf>, WatcherPathMatch)> {
        let mut results = Vec::new();

        if let Some(watcher_paths) = Weak::upgrade(&watcher_paths) {
            for wp in watcher_paths.lock().await.iter_mut() {
                let mut wp_paths = Vec::new();
                for path in paths {
                    if wp.applies_to_path(path) {
                        wp_paths.push(path);
                    }
                }
                if !wp_paths.is_empty() {
                    results.push((
                        wp_paths,
                        WatcherPathMatch {
                            only: wp.only.clone(),
                            excpet: wp.excpet.clone(),
                            reply: wp.reply.clone(),
                        },
                    ));
                }
            }
        }

        results
    }
}

struct WatcherPathMatch {
    pub only: ChangeKindSet,
    pub except: ChangeKindSet,
    pub reply: QueuedServerReply<DistantResponseData>,
}
