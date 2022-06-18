use crate::data::{Change, ChangeKind, ChangeKindSet, DistantResponseData, Error};
use distant_net::QueuedServerReply;
use std::{
    fmt,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
};

/// Represents a path registered with a watcher that includes relevant state including
/// the ability to reply with
#[derive(Clone)]
pub struct RegisteredPath {
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

    /// Specific filter for path
    except: ChangeKindSet,

    /// Used to send a reply through the connection watching this path
    reply: QueuedServerReply<DistantResponseData>,
}

impl fmt::Debug for RegisteredPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredPath")
            .field("raw_path", &self.raw_path)
            .field("path", &self.path)
            .field("recursive", &self.recursive)
            .field("only", &self.only)
            .field("except", &self.except)
            .finish()
    }
}

impl PartialEq for RegisteredPath {
    /// Checks for equality using the canonicalized path
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for RegisteredPath {}

impl Hash for RegisteredPath {
    /// Hashes using the canonicalized path
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl RegisteredPath {
    /// Registers a new path to be watched (does not actually do any watching)
    pub async fn register(
        path: impl Into<PathBuf>,
        recursive: bool,
        only: impl Into<ChangeKindSet>,
        except: impl Into<ChangeKindSet>,
        reply: QueuedServerReply<DistantResponseData>,
    ) -> io::Result<Self> {
        let raw_path = path.into();
        let path = tokio::fs::canonicalize(raw_path.as_path()).await?;
        let only = only.into();
        let except = except.into();
        Ok(Self {
            raw_path,
            path,
            recursive,
            only,
            except,
            reply,
        })
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

    /// Sends a reply for a change tied to this registered path, filtering
    /// out any paths that are not applicable
    ///
    /// Returns true if message was sent, and false if not
    pub async fn filter_and_send<T>(&self, kind: ChangeKind, paths: T) -> io::Result<bool>
    where
        T: IntoIterator,
        T::Item: AsRef<Path>,
    {
        let skip = (!self.only.is_empty() && !self.only.contains(&kind))
            || (!self.except.is_empty() && self.except.contains(&kind));

        if skip {
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

    /// Sends an error message an includes paths if provided
    pub async fn send_error<T>(&self, msg: &str, paths: T) -> io::Result<()>
    where
        T: IntoIterator,
        T::Item: AsRef<Path>,
    {
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|p| self.applies_to_path(p.as_ref()))
            .map(|p| p.as_ref().to_path_buf())
            .collect();

        self.reply
            .send(if paths.is_empty() {
                DistantResponseData::Error(Error::from(msg))
            } else {
                DistantResponseData::Error(Error::from(format!("{} about {:?}", msg, paths)))
            })
            .await
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
