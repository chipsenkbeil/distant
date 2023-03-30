use crate::data::{Change, ChangeKind, ChangeKindSet, DistantResponseData, Error};
use distant_net::common::ConnectionId;
use distant_net::server::Reply;
use std::{
    fmt,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
};

/// Represents a path registered with a watcher that includes relevant state including
/// the ability to reply with
pub struct RegisteredPath {
    /// Unique id tied to the path to distinguish it
    id: ConnectionId,

    /// The raw path provided to the watcher, which is not canonicalized
    raw_path: PathBuf,

    /// The canonicalized path at the time of providing to the watcher,
    /// as all paths must exist for a watcher, we use this to get the
    /// source of truth when watching
    path: PathBuf,

    /// Whether or not the path was set to be recursive
    recursive: bool,

    /// Specific filter for path (only the allowed change kinds are tracked)
    /// NOTE: This is a combination of only and except filters
    allowed: ChangeKindSet,

    /// Used to send a reply through the connection watching this path
    reply: Box<dyn Reply<Data = DistantResponseData>>,
}

impl fmt::Debug for RegisteredPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredPath")
            .field("raw_path", &self.raw_path)
            .field("path", &self.path)
            .field("recursive", &self.recursive)
            .field("allowed", &self.allowed)
            .finish()
    }
}

impl PartialEq for RegisteredPath {
    /// Checks for equality using the id, canonicalized path, and allowed change kinds
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.path == other.path && self.allowed == other.allowed
    }
}

impl Eq for RegisteredPath {}

impl Hash for RegisteredPath {
    /// Hashes using the id, canonicalized path, and allowed change kinds
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.path.hash(state);
        self.allowed.hash(state);
    }
}

impl RegisteredPath {
    /// Registers a new path to be watched (does not actually do any watching)
    pub async fn register(
        id: ConnectionId,
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
        except: impl Into<ChangeKindSet>,
        reply: Box<dyn Reply<Data = DistantResponseData>>,
    ) -> io::Result<Self> {
        let raw_path = path.into();
        let path = tokio::fs::canonicalize(raw_path.as_path()).await?;
        let only = only.into();
        let except = except.into();

        // Calculate the true list of kinds based on only and except filters
        let allowed = if only.is_empty() {
            ChangeKindSet::all() - except
        } else {
            only - except
        };

        Ok(Self {
            id,
            raw_path,
            path,
            recursive,
            allowed,
            reply,
        })
    }

    /// Represents a unique id to distinguish this path from other registrations
    /// of the same path
    pub fn id(&self) -> ConnectionId {
        self.id
    }

    /// Represents the path provided during registration before canonicalization
    pub fn raw_path(&self) -> &Path {
        self.raw_path.as_path()
    }

    /// Represents the canonicalized path used by watchers
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Returns true if this path represents a recursive watcher path
    pub fn is_recursive(&self) -> bool {
        self.recursive
    }

    /// Returns reference to set of [`ChangeKind`] that this path watches
    pub fn allowed(&self) -> &ChangeKindSet {
        &self.allowed
    }

    /// Sends a reply for a change tied to this registered path, filtering
    /// out any paths that are not applicable
    ///
    /// Returns true if message was sent, and false if not
    pub async fn filter_and_send<T>(&self, kind: ChangeKind, paths: T) -> io::Result<bool>
    where
        T: IntoIterator,
        T::Item: AsRef<Path>,
    {
        if !self.allowed().contains(&kind) {
            return Ok(false);
        }

        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| self.applies_to_path(p.as_ref()))
            .map(|p| p.as_ref().to_path_buf())
            .collect();

        if !paths.is_empty() {
            self.reply
                .send(DistantResponseData::Changed(Change { kind, paths }))
                .await
                .map(|_| true)
        } else {
            Ok(false)
        }
    }

    /// Sends an error message and includes paths if provided, skipping sending the message if
    /// no paths match and `skip_if_no_paths` is true
    ///
    /// Returns true if message was sent, and false if not
    pub async fn filter_and_send_error<T>(
        &self,
        msg: &str,
        paths: T,
        skip_if_no_paths: bool,
    ) -> io::Result<bool>
    where
        T: IntoIterator,
        T::Item: AsRef<Path>,
    {
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| self.applies_to_path(p.as_ref()))
            .map(|p| p.as_ref().to_path_buf())
            .collect();

        if !paths.is_empty() || !skip_if_no_paths {
            self.reply
                .send(if paths.is_empty() {
                    DistantResponseData::Error(Error::from(msg))
                } else {
                    DistantResponseData::Error(Error::from(format!("{msg} about {paths:?}")))
                })
                .await
                .map(|_| true)
        } else {
            Ok(false)
        }
    }

    /// Returns true if this path applies to the given path.
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
