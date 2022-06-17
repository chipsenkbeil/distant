use crate::{constants::SERVER_WATCHER_CAPACITY, data::ChangeKindSet};
use log::*;
use notify::{Config as WatcherConfig, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    hash::{Hash, Hasher},
    io,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};
use tokio::sync::mpsc::{self, error::TrySendError};

pub struct WatcherState {
    watcher: Option<RecommendedWatcher>,
}

impl WatcherState {
    pub fn new() -> Self {}

    pub fn is_initialized(&self) -> bool {
        self.watcher.is_some()
    }

    pub async fn initialize(&mut self) {
        if self.is_initialized() {
            return;
        }

        // NOTE: Cannot be something small like 1 as this seems to cause a deadlock sometimes
        //       with a large volume of watch requests
        let (tx, mut rx) = mpsc::channel(SERVER_WATCHER_CAPACITY);

        let mut watcher = notify::recommended_watcher(move |res| match tx.try_send(res) {
            Ok(_) => {}
            Err(TrySendError::Full(_)) => {
                warn!(
                    "Reached watcher capacity of {}! Dropping watcher event!",
                    SERVER_WATCHER_CAPACITY,
                );
            }
            Err(TrySendError::Closed(_)) => {
                warn!("Skipping watch event because watcher channel closed");
            }
        })?;

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

        self.watcher.replace();

        tokio::spawn(async move {
            while let Some(res) = rx.recv().await {
                let is_ok = match res {
                    Ok(mut x) => {
                        let paths: Vec<_> = x.paths.drain(..).collect();
                        let kind = ChangeKind::from(x.kind);

                        trace!(
                            "[Conn {}] Watcher detected '{}' change for {:?}",
                            conn_id,
                            kind,
                            paths
                        );

                        fn make_res_data(
                            kind: ChangeKind,
                            paths: &[&PathBuf],
                        ) -> DistantResponseData {
                            DistantResponseData::Changed(Change {
                                kind,
                                paths: paths.iter().map(|p| p.to_path_buf()).collect(),
                            })
                        }

                        let results = state.map_paths_to_watcher_paths_and_replies(&paths);
                        let mut is_ok = true;

                        for (paths, only, reply) in results {
                            // Skip sending this change if we are not watching it
                            if (!only.is_empty() && !only.contains(&kind))
                                || (!except.is_empty() && except.contains(&kind))
                            {
                                trace!(
                                    "[Conn {}] Skipping change '{}' for {:?}",
                                    conn_id,
                                    kind,
                                    paths
                                );
                                continue;
                            }

                            if !reply(vec![make_res_data(kind, &paths)]).await {
                                is_ok = false;
                                break;
                            }
                        }
                        is_ok
                    }
                    Err(mut x) => {
                        let paths: Vec<_> = x.paths.drain(..).collect();
                        let msg = x.to_string();

                        error!(
                            "[Conn {}] Watcher encountered an error {} for {:?}",
                            conn_id, msg, paths
                        );

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
                        if paths.is_empty() {
                            trace!("Relaying error to all watching connections");
                            for reply in state.watcher_paths.values_mut() {
                                if !reply(vec![make_res_data(&msg, &[])]).await {
                                    is_ok = false;
                                    break;
                                }
                            }
                        // Otherwise, figure out the relevant watchers from our paths and
                        // send the error to them
                        } else {
                            let results = state.map_paths_to_watcher_paths_and_replies(&paths);

                            trace!("Relaying error to {} watchers", results.len());
                            for (paths, _, reply) in results {
                                if !reply(vec![make_res_data(&msg, &paths)]).await {
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
}

#[derive(Clone, Debug)]
pub struct WatcherPath {
    /// The raw path provided to the watcher, which is not canonicalized
    raw_path: PathBuf,

    /// The canonicalized path at the time of providing to the watcher,
    /// as all paths must exist for a watcher, we use this to get the
    /// source of truth when watching
    path: PathBuf,

    /// Whether or not the path was set to be recursive
    recursive: bool,

    /// Specific filter for path
    only: ChangeKindSet,
}

impl PartialEq for WatcherPath {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for WatcherPath {}

impl Hash for WatcherPath {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl Deref for WatcherPath {
    type Target = PathBuf;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl DerefMut for WatcherPath {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.path
    }
}

impl WatcherPath {
    /// Create a new watcher path using the given path and canonicalizing it
    pub fn new(
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
    ) -> io::Result<Self> {
        let raw_path = path.into();
        let path = raw_path.canonicalize()?;
        let only = only.into();
        Ok(Self {
            raw_path,
            path,
            recursive,
            only,
        })
    }

    pub fn raw_path(&self) -> &Path {
        self.raw_path.as_path()
    }

    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Returns true if this watcher path applies to the given path.
    /// This is accomplished by checking if the path is contained
    /// within either the raw or canonicalized path of the watcher
    /// and ensures that recursion rules are respected
    pub fn applies_to_path(&self, path: &Path) -> bool {
        let check_path = |path: &Path| -> bool {
            let cnt = path.components().count();

            // 0 means exact match from strip_prefix
            // 1 means that it was within immediate directory (fine for non-recursive)
            // 2+ means it needs to be recursive
            cnt < 2 || self.recursive
        };

        match (
            path.strip_prefix(self.path()),
            path.strip_prefix(self.raw_path()),
        ) {
            (Ok(p1), Ok(p2)) => check_path(p1) || check_path(p2),
            (Ok(p), Err(_)) => check_path(p),
            (Err(_), Ok(p)) => check_path(p),
            (Err(_), Err(_)) => false,
        }
    }
}
