//! TTL-based caches with LRU eviction for mount filesystem metadata and content.
//!
//! Provides three cache types used by the mount translation layer to reduce
//! round-trips to the remote server:
//!
//! - [`AttrCache`] — file and directory metadata
//! - [`DirCache`] — directory listings
//! - [`ReadCache`] — file content by inode

use std::num::NonZeroUsize;
use std::time::{Duration, Instant, SystemTime};

use lru::LruCache;

use distant_core::protocol::{FileType, Metadata, RemotePath};

/// FUSE-compatible file attributes derived from remote [`Metadata`].
#[derive(Clone, Debug)]
pub struct FileAttr {
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    pub atime: SystemTime,
    pub mtime: SystemTime,
    pub ctime: SystemTime,
    pub kind: FileType,
    pub perm: u16,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
}

impl FileAttr {
    /// Converts remote [`Metadata`] into a [`FileAttr`] suitable for FUSE responses.
    ///
    /// Timestamps default to `UNIX_EPOCH` when not present in the metadata.
    /// Permissions are derived from unix metadata when available; otherwise
    /// read-only files get `0o444` and writable files get `0o644` (or `0o755`
    /// for directories).
    pub fn from_metadata(ino: u64, metadata: &Metadata) -> Self {
        let size = metadata.len;

        // 512-byte blocks, rounded up
        let blocks = size.div_ceil(512);

        let atime = metadata
            .accessed
            .map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let mtime = metadata
            .modified
            .map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let ctime = metadata
            .created
            .map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s))
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let perm = if let Some(ref unix) = metadata.unix {
            let mut mode: u16 = 0;
            if unix.owner_read {
                mode |= 0o400;
            }
            if unix.owner_write {
                mode |= 0o200;
            }
            if unix.owner_exec {
                mode |= 0o100;
            }
            if unix.group_read {
                mode |= 0o040;
            }
            if unix.group_write {
                mode |= 0o020;
            }
            if unix.group_exec {
                mode |= 0o010;
            }
            if unix.other_read {
                mode |= 0o004;
            }
            if unix.other_write {
                mode |= 0o002;
            }
            if unix.other_exec {
                mode |= 0o001;
            }
            mode
        } else if metadata.readonly {
            if metadata.file_type == FileType::Dir {
                0o555
            } else {
                0o444
            }
        } else if metadata.file_type == FileType::Dir {
            0o755
        } else {
            0o644
        };

        let nlink = if metadata.file_type == FileType::Dir {
            2
        } else {
            1
        };

        FileAttr {
            ino,
            size,
            blocks,
            atime,
            mtime,
            ctime,
            kind: metadata.file_type,
            perm,
            nlink,
            uid: 0,
            gid: 0,
        }
    }
}

/// Cached metadata entry pairing the raw remote [`Metadata`] with a
/// pre-computed [`FileAttr`].
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct CachedAttr {
    pub metadata: Metadata,
    pub attr: FileAttr,
}

/// A single entry within a cached directory listing.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct DirCacheEntry {
    pub name: String,
    pub ino: u64,
    pub file_type: FileType,
}

/// An LRU entry that tracks when it was inserted for TTL expiration.
struct TimedEntry<V> {
    value: V,
    inserted_at: Instant,
}

/// Cache for file and directory metadata, keyed by remote path.
///
/// Entries expire after a configurable TTL and are evicted in LRU order
/// when the cache exceeds its capacity.
pub struct AttrCache {
    inner: LruCache<RemotePath, TimedEntry<CachedAttr>>,
    ttl: Duration,
}

impl AttrCache {
    /// Creates a new attribute cache.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let cap = NonZeroUsize::new(capacity).expect("cache capacity must be non-zero");
        Self {
            inner: LruCache::new(cap),
            ttl,
        }
    }

    /// Returns a reference to the cached attributes for `key`, or `None` if
    /// absent or expired. Expired entries are removed on access.
    pub fn get(&mut self, key: &RemotePath) -> Option<&CachedAttr> {
        let expired = self
            .inner
            .peek(key)
            .is_some_and(|entry| entry.inserted_at.elapsed() >= self.ttl);

        if expired {
            self.inner.pop(key);
            return None;
        }

        self.inner.get(key).map(|entry| &entry.value)
    }

    /// Inserts or replaces the cached attributes for `key`.
    pub fn insert(&mut self, key: RemotePath, value: CachedAttr) {
        self.inner.put(
            key,
            TimedEntry {
                value,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Removes the entry for `key` if present.
    pub fn invalidate(&mut self, key: &RemotePath) {
        self.inner.pop(key);
    }

    /// Removes all entries from the cache.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

/// Cache for directory listings, keyed by the directory's remote path.
///
/// Entries expire after a configurable TTL and are evicted in LRU order
/// when the cache exceeds its capacity.
pub struct DirCache {
    inner: LruCache<RemotePath, TimedEntry<Vec<DirCacheEntry>>>,
    ttl: Duration,
}

impl DirCache {
    /// Creates a new directory cache.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let cap = NonZeroUsize::new(capacity).expect("cache capacity must be non-zero");
        Self {
            inner: LruCache::new(cap),
            ttl,
        }
    }

    /// Returns a reference to the cached directory listing for `key`, or
    /// `None` if absent or expired. Expired entries are removed on access.
    pub fn get(&mut self, key: &RemotePath) -> Option<&Vec<DirCacheEntry>> {
        let expired = self
            .inner
            .peek(key)
            .is_some_and(|entry| entry.inserted_at.elapsed() >= self.ttl);

        if expired {
            self.inner.pop(key);
            return None;
        }

        self.inner.get(key).map(|entry| &entry.value)
    }

    /// Inserts or replaces the cached directory listing for `key`.
    pub fn insert(&mut self, key: RemotePath, value: Vec<DirCacheEntry>) {
        self.inner.put(
            key,
            TimedEntry {
                value,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Removes the entry for `key` if present.
    pub fn invalidate(&mut self, key: &RemotePath) {
        self.inner.pop(key);
    }

    /// Removes all entries from the cache.
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

/// Cache for file contents, keyed by inode number.
///
/// Entries expire after a configurable TTL and are evicted in LRU order
/// when the cache exceeds its capacity. Intended for caching file reads
/// to avoid repeated round-trips for recently accessed content.
pub struct ReadCache {
    inner: LruCache<u64, TimedEntry<Vec<u8>>>,
    ttl: Duration,
}

impl ReadCache {
    /// Creates a new read cache.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        let cap = NonZeroUsize::new(capacity).expect("cache capacity must be non-zero");
        Self {
            inner: LruCache::new(cap),
            ttl,
        }
    }

    /// Returns a reference to the cached file contents for `ino`, or `None`
    /// if absent or expired. Expired entries are removed on access.
    pub fn get(&mut self, ino: &u64) -> Option<&Vec<u8>> {
        let expired = self
            .inner
            .peek(ino)
            .is_some_and(|entry| entry.inserted_at.elapsed() >= self.ttl);

        if expired {
            self.inner.pop(ino);
            return None;
        }

        self.inner.get(ino).map(|entry| &entry.value)
    }

    /// Inserts or replaces the cached file contents for `ino`.
    pub fn insert(&mut self, ino: u64, value: Vec<u8>) {
        self.inner.put(
            ino,
            TimedEntry {
                value,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Removes the entry for `ino` if present.
    pub fn invalidate(&mut self, ino: &u64) {
        self.inner.pop(ino);
    }

    /// Removes all entries from the cache.
    pub fn clear(&mut self) {
        self.inner.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    fn sample_metadata() -> Metadata {
        Metadata {
            canonicalized_path: None,
            file_type: FileType::File,
            len: 1024,
            readonly: false,
            accessed: Some(1_700_000_000),
            created: Some(1_690_000_000),
            modified: Some(1_695_000_000),
            unix: None,
            windows: None,
        }
    }

    mod attr_cache {
        use super::*;

        #[test]
        fn get_should_return_inserted_entry() {
            let mut cache = AttrCache::new(16, Duration::from_secs(60));
            let path = RemotePath::new("/test/file.txt");
            let meta = sample_metadata();
            let attr = FileAttr::from_metadata(42, &meta);
            let cached = CachedAttr {
                metadata: meta,
                attr,
            };

            cache.insert(path.clone(), cached);

            let result = cache.get(&path);
            assert!(result.is_some());
            let entry = result.expect("entry should be present");
            assert_eq!(entry.attr.ino, 42);
            assert_eq!(entry.attr.size, 1024);
        }

        #[test]
        fn get_should_return_none_for_missing_key() {
            let mut cache = AttrCache::new(16, Duration::from_secs(60));
            let path = RemotePath::new("/nonexistent");

            assert!(cache.get(&path).is_none());
        }

        #[test]
        fn get_should_return_none_after_ttl_expires() {
            let mut cache = AttrCache::new(16, Duration::from_millis(50));
            let path = RemotePath::new("/test/file.txt");
            let meta = sample_metadata();
            let attr = FileAttr::from_metadata(1, &meta);
            let cached = CachedAttr {
                metadata: meta,
                attr,
            };

            cache.insert(path.clone(), cached);
            assert!(cache.get(&path).is_some());

            thread::sleep(Duration::from_millis(60));

            assert!(cache.get(&path).is_none());
        }

        #[test]
        fn invalidate_should_remove_entry() {
            let mut cache = AttrCache::new(16, Duration::from_secs(60));
            let path = RemotePath::new("/test/file.txt");
            let meta = sample_metadata();
            let attr = FileAttr::from_metadata(1, &meta);
            let cached = CachedAttr {
                metadata: meta,
                attr,
            };

            cache.insert(path.clone(), cached);
            assert!(cache.get(&path).is_some());

            cache.invalidate(&path);
            assert!(cache.get(&path).is_none());
        }

        #[test]
        fn clear_should_remove_all_entries() {
            let mut cache = AttrCache::new(16, Duration::from_secs(60));

            for i in 0..5 {
                let path = RemotePath::new(format!("/file{i}"));
                let meta = sample_metadata();
                let attr = FileAttr::from_metadata(i, &meta);
                let cached = CachedAttr {
                    metadata: meta,
                    attr,
                };
                cache.insert(path, cached);
            }

            cache.clear();

            for i in 0..5 {
                let path = RemotePath::new(format!("/file{i}"));
                assert!(cache.get(&path).is_none());
            }
        }
    }

    mod dir_cache {
        use super::*;

        #[test]
        fn get_should_return_inserted_listing() {
            let mut cache = DirCache::new(16, Duration::from_secs(60));
            let path = RemotePath::new("/test/dir");
            let entries = vec![
                DirCacheEntry {
                    name: "foo.txt".to_string(),
                    ino: 10,
                    file_type: FileType::File,
                },
                DirCacheEntry {
                    name: "bar".to_string(),
                    ino: 11,
                    file_type: FileType::Dir,
                },
            ];

            cache.insert(path.clone(), entries);

            let result = cache.get(&path);
            assert!(result.is_some());
            let listing = result.expect("listing should be present");
            assert_eq!(listing.len(), 2);
            assert_eq!(listing[0].name, "foo.txt");
            assert_eq!(listing[1].name, "bar");
        }

        #[test]
        fn get_should_return_none_after_ttl_expires() {
            let mut cache = DirCache::new(16, Duration::from_millis(50));
            let path = RemotePath::new("/test/dir");
            let entries = vec![DirCacheEntry {
                name: "child".to_string(),
                ino: 5,
                file_type: FileType::File,
            }];

            cache.insert(path.clone(), entries);
            assert!(cache.get(&path).is_some());

            thread::sleep(Duration::from_millis(60));

            assert!(cache.get(&path).is_none());
        }

        #[test]
        fn invalidate_should_remove_entry() {
            let mut cache = DirCache::new(16, Duration::from_secs(60));
            let path = RemotePath::new("/test/dir");
            let entries = vec![DirCacheEntry {
                name: "child".to_string(),
                ino: 5,
                file_type: FileType::File,
            }];

            cache.insert(path.clone(), entries);
            cache.invalidate(&path);
            assert!(cache.get(&path).is_none());
        }
    }

    mod read_cache {
        use super::*;

        #[test]
        fn get_should_return_inserted_content() {
            let mut cache = ReadCache::new(16, Duration::from_secs(60));
            let content = b"hello world".to_vec();

            cache.insert(42, content);

            let result = cache.get(&42);
            assert!(result.is_some());
            assert_eq!(result.expect("content should be present"), b"hello world");
        }

        #[test]
        fn get_should_return_none_after_ttl_expires() {
            let mut cache = ReadCache::new(16, Duration::from_millis(50));

            cache.insert(42, b"data".to_vec());
            assert!(cache.get(&42).is_some());

            thread::sleep(Duration::from_millis(60));

            assert!(cache.get(&42).is_none());
        }

        #[test]
        fn invalidate_should_remove_entry() {
            let mut cache = ReadCache::new(16, Duration::from_secs(60));

            cache.insert(42, b"data".to_vec());
            cache.invalidate(&42);
            assert!(cache.get(&42).is_none());
        }

        #[test]
        fn clear_should_remove_all_entries() {
            let mut cache = ReadCache::new(16, Duration::from_secs(60));

            for i in 0..5 {
                cache.insert(i, vec![i as u8; 64]);
            }

            cache.clear();

            for i in 0..5 {
                assert!(cache.get(&i).is_none());
            }
        }
    }

    mod metadata_to_attr_conversion {
        use distant_core::protocol::UnixMetadata;

        use super::*;

        #[test]
        fn should_compute_blocks_from_size() {
            let meta = Metadata {
                len: 1000,
                ..sample_metadata()
            };

            let attr = FileAttr::from_metadata(1, &meta);

            // ceil(1000 / 512) = 2
            assert_eq!(attr.blocks, 2);
        }

        #[test]
        fn should_use_unix_epoch_when_timestamps_missing() {
            let meta = Metadata {
                accessed: None,
                created: None,
                modified: None,
                ..sample_metadata()
            };

            let attr = FileAttr::from_metadata(1, &meta);

            assert_eq!(attr.atime, SystemTime::UNIX_EPOCH);
            assert_eq!(attr.mtime, SystemTime::UNIX_EPOCH);
            assert_eq!(attr.ctime, SystemTime::UNIX_EPOCH);
        }

        #[test]
        fn should_convert_unix_permissions() {
            let meta = Metadata {
                unix: Some(UnixMetadata {
                    owner_read: true,
                    owner_write: true,
                    owner_exec: false,
                    group_read: true,
                    group_write: false,
                    group_exec: false,
                    other_read: true,
                    other_write: false,
                    other_exec: false,
                }),
                ..sample_metadata()
            };

            let attr = FileAttr::from_metadata(1, &meta);

            assert_eq!(attr.perm, 0o644);
        }

        #[test]
        fn should_use_fallback_perms_for_readonly_file() {
            let meta = Metadata {
                readonly: true,
                file_type: FileType::File,
                ..sample_metadata()
            };

            let attr = FileAttr::from_metadata(1, &meta);

            assert_eq!(attr.perm, 0o444);
        }

        #[test]
        fn should_use_fallback_perms_for_readonly_dir() {
            let meta = Metadata {
                readonly: true,
                file_type: FileType::Dir,
                ..sample_metadata()
            };

            let attr = FileAttr::from_metadata(1, &meta);

            assert_eq!(attr.perm, 0o555);
        }

        #[test]
        fn should_set_nlink_to_2_for_directories() {
            let meta = Metadata {
                file_type: FileType::Dir,
                ..sample_metadata()
            };

            let attr = FileAttr::from_metadata(1, &meta);

            assert_eq!(attr.nlink, 2);
        }

        #[test]
        fn should_set_nlink_to_1_for_files() {
            let attr = FileAttr::from_metadata(1, &sample_metadata());

            assert_eq!(attr.nlink, 1);
        }
    }
}
