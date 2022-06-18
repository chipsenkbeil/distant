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

    /// Specific filter for path
    except: ChangeKindSet,

    /// Used to send a reply through the connection watching this path
    reply: QueuedServerReply<DistantResponseData>,
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
        except: impl Into<ChangeKindSet>,
        reply: QueuedServerReply<DistantResponseData>,
    ) -> io::Result<Self> {
        let raw_path = path.into();
        let path = raw_path.canonicalize()?;
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
