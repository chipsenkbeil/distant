//! Bidirectional mapping between inode numbers and remote paths.
//!
//! The [`InodeTable`] maintains O(1) lookups in both directions (inode to path
//! and path to inode), reference counting for active handles, and LRU eviction
//! of zero-refcount inodes when the table exceeds a configurable capacity.
//!
//! Used by FUSE and NFS backends to translate between kernel inode numbers and
//! the [`RemotePath`] values expected by the distant protocol.

use std::collections::{HashMap, VecDeque};

use distant_core::protocol::RemotePath;

/// Inode number reserved for the filesystem root.
const ROOT_INO: u64 = 1;

/// A bidirectional inode-to-path table with reference counting and LRU eviction.
///
/// Every mounted filesystem starts with a root inode (inode 1) mapped to the
/// configured remote root path. New inodes are allocated monotonically. When an
/// inode's reference count drops to zero it becomes eligible for eviction; once
/// the table exceeds its capacity, the least-recently-zeroed inodes are removed.
pub(crate) struct InodeTable {
    next_ino: u64,
    by_ino: HashMap<u64, InodeEntry>,
    by_path: HashMap<RemotePath, u64>,
    capacity: usize,
    lru: VecDeque<u64>,
}

struct InodeEntry {
    path: RemotePath,
    refcount: u64,
}

impl InodeTable {
    /// Creates a new inode table with the given remote root path and maximum
    /// capacity.
    ///
    /// The root inode (inode 1) is pre-allocated with a reference count of 1.
    /// `capacity` controls how many zero-refcount inodes are retained before
    /// LRU eviction begins.
    pub fn new(root: RemotePath, capacity: usize) -> Self {
        let mut by_ino = HashMap::new();
        let mut by_path = HashMap::new();

        by_ino.insert(
            ROOT_INO,
            InodeEntry {
                path: root.clone(),
                refcount: 1,
            },
        );
        by_path.insert(root, ROOT_INO);

        Self {
            next_ino: ROOT_INO + 1,
            by_ino,
            by_path,
            capacity,
            lru: VecDeque::new(),
        }
    }

    /// Returns the path associated with the given inode, or `None` if the inode
    /// is not in the table.
    pub fn get_path(&self, ino: u64) -> Option<RemotePath> {
        self.by_ino.get(&ino).map(|entry| entry.path.clone())
    }

    /// Returns the inode number associated with the given path, or `None` if
    /// the path is not in the table.
    pub fn get_ino(&self, path: &RemotePath) -> Option<u64> {
        self.by_path.get(path).copied()
    }

    /// Inserts a path into the table and returns its inode number.
    ///
    /// If the path already exists in the table, its existing inode number is
    /// returned without allocating a new one. The reference count is not
    /// modified; callers should use [`inc_ref`](Self::inc_ref) to track active
    /// handles.
    pub fn insert(&mut self, path: RemotePath) -> u64 {
        if let Some(&ino) = self.by_path.get(&path) {
            return ino;
        }

        let ino = self.next_ino;
        self.next_ino += 1;

        self.by_ino.insert(
            ino,
            InodeEntry {
                path: path.clone(),
                refcount: 0,
            },
        );
        self.by_path.insert(path, ino);

        // A newly inserted inode starts with refcount 0, so it is immediately
        // eligible for eviction.
        self.lru.push_back(ino);
        self.evict();

        ino
    }

    /// Looks up a child entry by name under the given parent inode.
    ///
    /// Constructs the full child path by joining the parent's path with `name`
    /// using a `/` separator, then looks up the result in the path index.
    /// Returns the child's inode number if it exists.
    pub fn lookup(&self, parent_ino: u64, name: &str) -> Option<u64> {
        let parent_path = self.get_path(parent_ino)?;
        let child_path = join_remote_path(&parent_path, name);
        self.get_ino(&child_path)
    }

    /// Increments the reference count for the given inode.
    ///
    /// If the inode was in the LRU eviction queue (refcount was 0), it is
    /// removed from the queue since it is no longer eligible for eviction.
    pub fn inc_ref(&mut self, ino: u64) {
        if let Some(entry) = self.by_ino.get_mut(&ino) {
            if entry.refcount == 0 {
                self.lru.retain(|&candidate| candidate != ino);
            }
            entry.refcount += 1;
        }
    }

    /// Decrements the reference count for the given inode by `count`.
    ///
    /// If the reference count reaches zero, the inode is added to the LRU
    /// eviction queue. Eviction runs automatically if the table exceeds its
    /// configured capacity.
    ///
    /// The reference count is clamped to zero (it will never underflow).
    pub fn dec_ref(&mut self, ino: u64, count: u64) {
        if let Some(entry) = self.by_ino.get_mut(&ino) {
            entry.refcount = entry.refcount.saturating_sub(count);
            if entry.refcount == 0 {
                self.lru.push_back(ino);
                self.evict();
            }
        }
    }

    /// Updates the path for an existing inode without changing its inode number.
    ///
    /// Both the forward (inode-to-path) and reverse (path-to-inode) maps are
    /// updated atomically. If the inode does not exist in the table, this is a
    /// no-op.
    pub fn rename(&mut self, ino: u64, new_path: RemotePath) {
        if let Some(entry) = self.by_ino.get_mut(&ino) {
            self.by_path.remove(&entry.path);
            entry.path = new_path.clone();
            self.by_path.insert(new_path, ino);
        }
    }

    /// Evicts the oldest zero-refcount inodes until the total table size is at
    /// or below the configured capacity.
    fn evict(&mut self) {
        while self.by_ino.len() > self.capacity {
            let Some(ino) = self.lru.pop_front() else {
                break;
            };

            // The inode may have been re-referenced since it was enqueued.
            let dominated = self
                .by_ino
                .get(&ino)
                .is_some_and(|entry| entry.refcount == 0);

            if dominated && let Some(entry) = self.by_ino.remove(&ino) {
                self.by_path.remove(&entry.path);
            }
        }
    }
}

/// Joins a parent remote path with a child name using `/` as the separator.
///
/// Handles the case where the parent path already ends with `/` to avoid
/// producing double slashes.
fn join_remote_path(parent: &RemotePath, name: &str) -> RemotePath {
    let parent_str = parent.as_str();
    if parent_str.ends_with('/') {
        RemotePath::new(format!("{parent_str}{name}"))
    } else {
        RemotePath::new(format!("{parent_str}/{name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_path() -> RemotePath {
        RemotePath::new("/remote/root")
    }

    #[test]
    fn root_inode_should_be_allocated_at_creation() {
        let table = InodeTable::new(root_path(), 100);

        assert_eq!(table.get_path(ROOT_INO), Some(root_path()));
        assert_eq!(table.get_ino(&root_path()), Some(ROOT_INO));
    }

    #[test]
    fn root_inode_should_have_refcount_one() {
        let mut table = InodeTable::new(root_path(), 100);

        // Decrementing the root's single reference should not panic or remove it
        // from the table (it goes to zero and enters the LRU queue, but
        // capacity is large enough to keep it).
        table.dec_ref(ROOT_INO, 1);
        assert_eq!(table.get_path(ROOT_INO), Some(root_path()));
    }

    #[test]
    fn insert_should_allocate_new_inode() {
        let mut table = InodeTable::new(root_path(), 100);

        let child = RemotePath::new("/remote/root/file.txt");
        let ino = table.insert(child.clone());

        assert_ne!(ino, ROOT_INO);
        assert_eq!(table.get_path(ino), Some(child.clone()));
        assert_eq!(table.get_ino(&child), Some(ino));
    }

    #[test]
    fn insert_should_return_existing_inode_for_same_path() {
        let mut table = InodeTable::new(root_path(), 100);

        let child = RemotePath::new("/remote/root/file.txt");
        let ino1 = table.insert(child.clone());
        let ino2 = table.insert(child);

        assert_eq!(ino1, ino2);
    }

    #[test]
    fn insert_should_assign_monotonically_increasing_inodes() {
        let mut table = InodeTable::new(root_path(), 100);

        let ino_a = table.insert(RemotePath::new("/a"));
        let ino_b = table.insert(RemotePath::new("/b"));
        let ino_c = table.insert(RemotePath::new("/c"));

        assert!(ino_a < ino_b);
        assert!(ino_b < ino_c);
    }

    #[test]
    fn lookup_should_find_existing_child() {
        let mut table = InodeTable::new(root_path(), 100);

        let child = RemotePath::new("/remote/root/file.txt");
        let ino = table.insert(child);

        assert_eq!(table.lookup(ROOT_INO, "file.txt"), Some(ino));
    }

    #[test]
    fn lookup_should_return_none_for_missing_child() {
        let table = InodeTable::new(root_path(), 100);

        assert_eq!(table.lookup(ROOT_INO, "nonexistent"), None);
    }

    #[test]
    fn lookup_should_return_none_for_missing_parent() {
        let table = InodeTable::new(root_path(), 100);

        assert_eq!(table.lookup(999, "anything"), None);
    }

    #[test]
    fn lookup_should_handle_trailing_slash_in_parent() {
        let mut table = InodeTable::new(RemotePath::new("/root/"), 100);

        let child = RemotePath::new("/root/child");
        let ino = table.insert(child);

        assert_eq!(table.lookup(ROOT_INO, "child"), Some(ino));
    }

    #[test]
    fn inc_ref_should_increase_refcount() {
        let mut table = InodeTable::new(root_path(), 100);

        let child = RemotePath::new("/remote/root/file.txt");
        let ino = table.insert(child.clone());

        table.inc_ref(ino);
        table.inc_ref(ino);

        // After two increments, decrementing by 1 should not make it eligible
        // for eviction.
        table.dec_ref(ino, 1);
        assert_eq!(table.get_path(ino), Some(child));
    }

    #[test]
    fn dec_ref_should_clamp_at_zero() {
        let mut table = InodeTable::new(root_path(), 100);

        let child = RemotePath::new("/remote/root/file.txt");
        let ino = table.insert(child.clone());

        // Decrementing more than the refcount should not panic.
        table.dec_ref(ino, 100);
        assert_eq!(table.get_path(ino), Some(child));
    }

    #[test]
    fn dec_ref_should_make_inode_evictable() {
        // Capacity of 3: root + two children fit exactly.
        let mut table = InodeTable::new(root_path(), 3);

        let child_a = RemotePath::new("/a");
        let child_b = RemotePath::new("/b");
        let child_c = RemotePath::new("/c");

        let ino_a = table.insert(child_a.clone());
        table.inc_ref(ino_a);

        let ino_b = table.insert(child_b.clone());
        table.inc_ref(ino_b);

        // Table: root(ref=1), a(ref=1), b(ref=1). Size=3 == capacity, all fit.
        assert_eq!(table.get_path(ino_a), Some(child_a.clone()));
        assert_eq!(table.get_path(ino_b), Some(child_b));

        // Drop reference on child_a, making it eligible for eviction.
        table.dec_ref(ino_a, 1);

        // Insert child_c — table now has 4 entries, exceeds capacity of 3.
        // child_a (refcount=0) is in the LRU queue and gets evicted.
        let ino_c = table.insert(child_c.clone());
        table.inc_ref(ino_c);

        assert_eq!(table.get_path(ino_a), None);
        assert_eq!(table.get_ino(&child_a), None);
        assert_eq!(table.get_path(ino_c), Some(child_c));
    }

    #[test]
    fn eviction_should_remove_lru_order_zero_refcount_inodes() {
        // Capacity of 2 means only 2 inodes total (including root).
        let mut table = InodeTable::new(root_path(), 2);

        let child_a = RemotePath::new("/a");
        let child_b = RemotePath::new("/b");

        // Insert child_a — table has 3 entries (root, a). Root was
        // initially refcount=1, child_a is refcount=0 and goes to LRU.
        // Since child_a is zero-refcount and table size (root + a = 2) is at
        // capacity, no eviction yet.
        let ino_a = table.insert(child_a.clone());

        // Both should still exist (size == capacity).
        assert_eq!(table.get_path(ROOT_INO), Some(root_path()));
        assert_eq!(table.get_path(ino_a), Some(child_a));

        // Insert child_b — table now has 3 entries, exceeds capacity of 2.
        // child_a was first in LRU, so it gets evicted.
        let ino_b = table.insert(child_b.clone());

        assert_eq!(table.get_path(ino_a), None);
        assert_eq!(table.get_ino(&RemotePath::new("/a")), None);
        assert_eq!(table.get_path(ino_b), Some(child_b));
    }

    #[test]
    fn eviction_should_preserve_referenced_inodes() {
        // Capacity of 3: root + two children fit exactly.
        let mut table = InodeTable::new(root_path(), 3);

        let child_a = RemotePath::new("/a");
        let child_b = RemotePath::new("/b");
        let child_c = RemotePath::new("/c");

        let ino_a = table.insert(child_a.clone());
        let ino_b = table.insert(child_b.clone());

        // Both start at refcount=0 in the LRU queue. Reference child_a to
        // protect it from eviction.
        table.inc_ref(ino_a);

        // Insert child_c — size becomes 4, exceeding capacity of 3. Only
        // child_b (refcount=0) is eligible for eviction.
        let ino_c = table.insert(child_c.clone());

        // child_a survived because it has a non-zero refcount.
        assert_eq!(table.get_path(ino_a), Some(child_a));

        // child_b was evicted (zero refcount, oldest eligible entry).
        assert_eq!(table.get_path(ino_b), None);

        // child_c was just inserted and remains (table back at capacity).
        assert_eq!(table.get_path(ino_c), Some(child_c));
    }

    #[test]
    fn rename_should_update_path_without_changing_inode() {
        let mut table = InodeTable::new(root_path(), 100);

        let old_path = RemotePath::new("/remote/root/old.txt");
        let new_path = RemotePath::new("/remote/root/new.txt");
        let ino = table.insert(old_path.clone());

        table.rename(ino, new_path.clone());

        assert_eq!(table.get_path(ino), Some(new_path.clone()));
        assert_eq!(table.get_ino(&new_path), Some(ino));
        assert_eq!(table.get_ino(&old_path), None);
    }

    #[test]
    fn rename_should_be_noop_for_unknown_inode() {
        let mut table = InodeTable::new(root_path(), 100);

        // Should not panic or corrupt state.
        table.rename(999, RemotePath::new("/nonexistent"));
        assert_eq!(table.get_path(ROOT_INO), Some(root_path()));
    }

    #[test]
    fn get_path_and_get_ino_should_be_consistent() {
        let mut table = InodeTable::new(root_path(), 100);

        let paths = vec![
            RemotePath::new("/a"),
            RemotePath::new("/b"),
            RemotePath::new("/c/d"),
        ];

        let inodes: Vec<u64> = paths.iter().map(|p| table.insert(p.clone())).collect();

        for (path, &ino) in paths.iter().zip(&inodes) {
            assert_eq!(table.get_path(ino), Some(path.clone()));
            assert_eq!(table.get_ino(path), Some(ino));
        }
    }

    #[test]
    fn get_path_should_return_none_for_unknown_inode() {
        let table = InodeTable::new(root_path(), 100);
        assert_eq!(table.get_path(999), None);
    }

    #[test]
    fn get_ino_should_return_none_for_unknown_path() {
        let table = InodeTable::new(root_path(), 100);
        assert_eq!(table.get_ino(&RemotePath::new("/unknown")), None);
    }
}
