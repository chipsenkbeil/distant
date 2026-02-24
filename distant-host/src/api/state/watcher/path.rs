use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::{fmt, io};

use distant_core::net::common::ConnectionId;
use distant_core::net::server::Reply;
use distant_core::protocol::{Change, ChangeKindSet, Error, Response};

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
    reply: Box<dyn Reply<Data = Response>>,
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
        reply: Box<dyn Reply<Data = Response>>,
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
    /// out any changes that are not applicable.
    ///
    /// Returns true if message was sent, and false if not.
    pub fn filter_and_send(&self, change: Change) -> io::Result<bool> {
        if !self.allowed().contains(&change.kind) {
            return Ok(false);
        }

        // Only send if this registered path applies to the changed path
        if self.applies_to_path(&change.path) {
            self.reply.send(Response::Changed(change)).map(|_| true)
        } else {
            Ok(false)
        }
    }

    /// Sends an error message and includes paths if provided, skipping sending the message if
    /// no paths match and `skip_if_no_paths` is true.
    ///
    /// Returns true if message was sent, and false if not.
    pub fn filter_and_send_error<T>(
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
                    Response::Error(Error::from(msg))
                } else {
                    Response::Error(Error::from(format!("{msg} about {paths:?}")))
                })
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

#[cfg(test)]
mod tests {
    //! Tests for `RegisteredPath` covering registration with various filter configurations,
    //! path applicability (recursive vs non-recursive), change event filtering and sending,
    //! error filtering, and equality/hash behavior.

    use super::*;

    use std::collections::hash_map::DefaultHasher;

    use distant_core::protocol::{ChangeDetails, ChangeKind};

    /// Simple test reply that captures sent responses via a std mpsc channel
    struct TestReply(std::sync::mpsc::Sender<Response>);

    impl Reply for TestReply {
        type Data = Response;

        fn send(&self, data: Response) -> io::Result<()> {
            self.0
                .send(data)
                .map_err(|e| io::Error::other(e.to_string()))
        }

        fn clone_reply(&self) -> Box<dyn Reply<Data = Response>> {
            Box::new(TestReply(self.0.clone()))
        }
    }

    /// Helper to create a test reply pair (reply box, receiver)
    fn test_reply() -> (
        Box<dyn Reply<Data = Response>>,
        std::sync::mpsc::Receiver<Response>,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Box::new(TestReply(tx)), rx)
    }

    /// Helper to compute a hash value for a RegisteredPath
    fn hash_of(rp: &RegisteredPath) -> u64 {
        let mut hasher = DefaultHasher::new();
        rp.hash(&mut hasher);
        hasher.finish()
    }

    mod register {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn with_valid_path_sets_all_fields() {
            let dir = tempfile::tempdir().unwrap();
            let dir_path = dir.path().to_path_buf();
            let (reply, _rx) = test_reply();

            let rp = RegisteredPath::register(
                42,
                dir_path.clone(),
                true,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            assert_eq!(rp.id(), 42);
            assert_eq!(rp.raw_path(), dir_path);
            // Canonicalized path should resolve to the real path
            assert!(rp.path().exists());
            assert!(rp.is_recursive());
            // With empty only and empty except, allowed should be all
            assert_eq!(*rp.allowed(), ChangeKindSet::all());
        }

        #[test_log::test(tokio::test)]
        async fn with_nonexistent_path_fails() {
            let (reply, _rx) = test_reply();
            let result = RegisteredPath::register(
                1,
                PathBuf::from("/nonexistent/path/that/does/not/exist"),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await;

            assert!(result.is_err());
        }

        #[test_log::test(tokio::test)]
        async fn with_only_filter_restricts_allowed() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, _rx) = test_reply();

            let only = ChangeKindSet::new([ChangeKind::Create, ChangeKind::Delete]);
            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                only,
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            assert!(rp.allowed().contains(&ChangeKind::Create));
            assert!(rp.allowed().contains(&ChangeKind::Delete));
            assert!(!rp.allowed().contains(&ChangeKind::Modify));
            assert!(!rp.allowed().contains(&ChangeKind::Access));
        }

        #[test_log::test(tokio::test)]
        async fn with_except_filter_removes_from_all() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, _rx) = test_reply();

            let except = ChangeKindSet::new([ChangeKind::Access]);
            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                except,
                reply,
            )
            .await
            .unwrap();

            assert!(!rp.allowed().contains(&ChangeKind::Access));
            assert!(rp.allowed().contains(&ChangeKind::Create));
            assert!(rp.allowed().contains(&ChangeKind::Modify));
        }

        #[test_log::test(tokio::test)]
        async fn with_only_and_except_filters_combined() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, _rx) = test_reply();

            let only = ChangeKindSet::new([ChangeKind::Create, ChangeKind::Delete]);
            let except = ChangeKindSet::new([ChangeKind::Delete]);
            let rp =
                RegisteredPath::register(1, dir.path().to_path_buf(), false, only, except, reply)
                    .await
                    .unwrap();

            assert!(rp.allowed().contains(&ChangeKind::Create));
            assert!(!rp.allowed().contains(&ChangeKind::Delete));
        }

        #[test_log::test(tokio::test)]
        async fn non_recursive_flag() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, _rx) = test_reply();

            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            assert!(!rp.is_recursive());
        }
    }

    mod applies_to_path {
        use super::*;

        /// Helper: register a path with default settings for applies_to_path testing
        async fn make_registered(dir: &Path, recursive: bool) -> RegisteredPath {
            let (reply, _rx) = test_reply();
            RegisteredPath::register(
                1,
                dir.to_path_buf(),
                recursive,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap()
        }

        #[test_log::test(tokio::test)]
        async fn exact_match_returns_true() {
            let dir = tempfile::tempdir().unwrap();
            let rp = make_registered(dir.path(), false).await;
            // The canonicalized path should match exactly
            assert!(rp.applies_to_path(rp.path()));
        }

        #[test_log::test(tokio::test)]
        async fn direct_child_returns_true_for_non_recursive() {
            let dir = tempfile::tempdir().unwrap();
            let rp = make_registered(dir.path(), false).await;
            let child = rp.path().join("child.txt");
            assert!(rp.applies_to_path(&child));
        }

        #[test_log::test(tokio::test)]
        async fn deep_child_returns_false_for_non_recursive() {
            let dir = tempfile::tempdir().unwrap();
            let rp = make_registered(dir.path(), false).await;
            let deep = rp.path().join("sub").join("deep.txt");
            assert!(!rp.applies_to_path(&deep));
        }

        #[test_log::test(tokio::test)]
        async fn deep_child_returns_true_for_recursive() {
            let dir = tempfile::tempdir().unwrap();
            let rp = make_registered(dir.path(), true).await;
            let deep = rp.path().join("sub").join("deep.txt");
            assert!(rp.applies_to_path(&deep));
        }

        #[test_log::test(tokio::test)]
        async fn unrelated_path_returns_false() {
            let dir = tempfile::tempdir().unwrap();
            let rp = make_registered(dir.path(), true).await;
            let unrelated = PathBuf::from("/completely/unrelated/path");
            assert!(!rp.applies_to_path(&unrelated));
        }
    }

    mod filter_and_send {
        use super::*;

        /// Helper to build a Change
        fn make_change(kind: ChangeKind, path: PathBuf) -> Change {
            Change {
                timestamp: 0,
                kind,
                path,
                details: ChangeDetails::default(),
            }
        }

        #[test_log::test(tokio::test)]
        async fn sends_matching_change() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, rx) = test_reply();

            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            let child = rp.path().join("file.txt");
            let change = make_change(ChangeKind::Create, child.clone());
            let sent = rp.filter_and_send(change.clone()).unwrap();
            assert!(sent);

            let response = rx.recv().unwrap();
            match response {
                Response::Changed(c) => {
                    assert_eq!(c.kind, ChangeKind::Create);
                    assert_eq!(c.path, child);
                }
                other => panic!("Expected Response::Changed, got {:?}", other),
            }
        }

        #[test_log::test(tokio::test)]
        async fn skips_change_with_disallowed_kind() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, rx) = test_reply();

            // Only allow Create changes
            let only = ChangeKindSet::new([ChangeKind::Create]);
            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                only,
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            let child = rp.path().join("file.txt");
            let change = make_change(ChangeKind::Delete, child);
            let sent = rp.filter_and_send(change).unwrap();
            assert!(!sent);

            // Nothing should have been sent
            assert!(rx.try_recv().is_err());
        }

        #[test_log::test(tokio::test)]
        async fn skips_change_for_unrelated_path() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, rx) = test_reply();

            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            let unrelated = PathBuf::from("/some/other/path/file.txt");
            let change = make_change(ChangeKind::Create, unrelated);
            let sent = rp.filter_and_send(change).unwrap();
            assert!(!sent);

            assert!(rx.try_recv().is_err());
        }
    }

    mod filter_and_send_error {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn sends_error_when_paths_match() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, rx) = test_reply();

            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            let child = rp.path().join("file.txt");
            let sent = rp
                .filter_and_send_error("test error", vec![child], true)
                .unwrap();
            assert!(sent);

            let response = rx.recv().unwrap();
            match response {
                Response::Error(e) => {
                    assert!(e.to_string().contains("test error"));
                    assert!(e.to_string().contains("about"));
                }
                other => panic!("Expected Response::Error, got {:?}", other),
            }
        }

        #[test_log::test(tokio::test)]
        async fn skips_when_no_paths_match_and_skip_is_true() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, rx) = test_reply();

            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            let unrelated = PathBuf::from("/unrelated/path");
            let sent = rp
                .filter_and_send_error("test error", vec![unrelated], true)
                .unwrap();
            assert!(!sent);

            assert!(rx.try_recv().is_err());
        }

        #[test_log::test(tokio::test)]
        async fn sends_when_no_paths_match_and_skip_is_false() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, rx) = test_reply();

            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            let unrelated = PathBuf::from("/unrelated/path");
            let sent = rp
                .filter_and_send_error("test error", vec![unrelated], false)
                .unwrap();
            assert!(sent);

            let response = rx.recv().unwrap();
            match response {
                Response::Error(e) => {
                    // When paths list is empty after filtering, the message should
                    // not contain "about"
                    assert!(e.to_string().contains("test error"));
                    assert!(!e.to_string().contains("about"));
                }
                other => panic!("Expected Response::Error, got {:?}", other),
            }
        }

        #[test_log::test(tokio::test)]
        async fn sends_with_empty_paths_and_skip_is_false() {
            let dir = tempfile::tempdir().unwrap();
            let (reply, rx) = test_reply();

            let rp = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply,
            )
            .await
            .unwrap();

            let sent = rp
                .filter_and_send_error("test error", Vec::<PathBuf>::new(), false)
                .unwrap();
            assert!(sent);

            let response = rx.recv().unwrap();
            match response {
                Response::Error(e) => {
                    assert!(e.to_string().contains("test error"));
                }
                other => panic!("Expected Response::Error, got {:?}", other),
            }
        }
    }

    mod equality_and_hash {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn partial_eq_uses_id_path_and_allowed() {
            let dir = tempfile::tempdir().unwrap();
            let (reply1, _rx1) = test_reply();
            let (reply2, _rx2) = test_reply();

            let rp1 = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                true,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply1,
            )
            .await
            .unwrap();

            // Same id, same path, same allowed, but different recursive flag
            let rp2 = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false, // different recursive
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply2,
            )
            .await
            .unwrap();

            // PartialEq does NOT use recursive, so these should be equal
            assert_eq!(rp1, rp2);
        }

        #[test_log::test(tokio::test)]
        async fn partial_eq_differs_on_id() {
            let dir = tempfile::tempdir().unwrap();
            let (reply1, _rx1) = test_reply();
            let (reply2, _rx2) = test_reply();

            let rp1 = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply1,
            )
            .await
            .unwrap();

            let rp2 = RegisteredPath::register(
                2, // different id
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply2,
            )
            .await
            .unwrap();

            assert_ne!(rp1, rp2);
        }

        #[test_log::test(tokio::test)]
        async fn partial_eq_differs_on_allowed() {
            let dir = tempfile::tempdir().unwrap();
            let (reply1, _rx1) = test_reply();
            let (reply2, _rx2) = test_reply();

            let rp1 = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::new([ChangeKind::Create]),
                ChangeKindSet::empty(),
                reply1,
            )
            .await
            .unwrap();

            let rp2 = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::new([ChangeKind::Delete]),
                ChangeKindSet::empty(),
                reply2,
            )
            .await
            .unwrap();

            assert_ne!(rp1, rp2);
        }

        #[test_log::test(tokio::test)]
        async fn hash_consistent_with_partial_eq() {
            let dir = tempfile::tempdir().unwrap();
            let (reply1, _rx1) = test_reply();
            let (reply2, _rx2) = test_reply();

            let rp1 = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                true,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply1,
            )
            .await
            .unwrap();

            let rp2 = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false, // different recursive, but PartialEq ignores it
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply2,
            )
            .await
            .unwrap();

            // Equal objects must have equal hashes
            assert_eq!(rp1, rp2);
            assert_eq!(hash_of(&rp1), hash_of(&rp2));
        }

        #[test_log::test(tokio::test)]
        async fn hash_differs_for_unequal_objects() {
            let dir = tempfile::tempdir().unwrap();
            let (reply1, _rx1) = test_reply();
            let (reply2, _rx2) = test_reply();

            let rp1 = RegisteredPath::register(
                1,
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply1,
            )
            .await
            .unwrap();

            let rp2 = RegisteredPath::register(
                2, // different id
                dir.path().to_path_buf(),
                false,
                ChangeKindSet::empty(),
                ChangeKindSet::empty(),
                reply2,
            )
            .await
            .unwrap();

            assert_ne!(rp1, rp2);
            // Different hashes (not strictly required but very likely with different ids)
            assert_ne!(hash_of(&rp1), hash_of(&rp2));
        }
    }
}
