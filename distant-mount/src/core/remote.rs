mod buffer;
mod cache;
mod inode;

use std::io;
use std::sync::Arc;

use log::{debug, warn};
use tokio::sync::{Mutex, RwLock};

use distant_core::protocol::{
    Change, ChangeKind, Metadata, ReadFileOptions, RemotePath, WriteFileOptions,
};
use distant_core::{Channel, ChannelExt};

use crate::core::MountConfig;
use buffer::WriteBuffers;
use cache::{AttrCache, CachedAttr, DirCache, DirCacheEntry, ReadCache};
use inode::InodeTable;

pub use cache::FileAttr;

/// Translates filesystem operations into distant [`ChannelExt`] calls.
///
/// Provides a async API for backends to leverage if they support Rust's native async interface.
#[allow(dead_code)]
pub struct RemoteFs {
    channel: Channel,
    inodes: RwLock<InodeTable>,
    attr_cache: Arc<Mutex<AttrCache>>,
    dir_cache: Arc<Mutex<DirCache>>,
    read_cache: Arc<Mutex<ReadCache>>,
    write_buffers: Mutex<WriteBuffers>,
    config: MountConfig,
    watch_handle: Option<tokio::task::JoinHandle<()>>,
}

impl RemoteFs {
    /// Creates a new `RemoteFs` connected to the given distant channel.
    ///
    /// If `config.remote_root` is `None`, the remote server's current working
    /// directory is fetched via `system_info` and used as the root path.
    ///
    /// # Errors
    ///
    /// Returns an error if the initial `system_info` call fails when no
    /// explicit remote root is configured.
    pub async fn init(channel: Channel, config: MountConfig) -> io::Result<Self> {
        let root_path = match config.remote_root {
            Some(ref path) => path.clone(),
            None => {
                let mut ch = channel.clone();
                let info = ch.system_info().await?;
                debug!("remote root defaulting to server cwd: {}", info.current_dir);
                info.current_dir
            }
        };

        let cache = &config.cache;

        let attr_cache = Arc::new(Mutex::new(AttrCache::new(
            cache.attr_capacity,
            cache.attr_ttl,
        )));
        let dir_cache = Arc::new(Mutex::new(DirCache::new(cache.dir_capacity, cache.dir_ttl)));
        let read_cache = Arc::new(Mutex::new(ReadCache::new(
            cache.read_capacity,
            cache.read_ttl,
        )));

        let watch_handle = spawn_watch_task(
            channel.clone(),
            root_path.clone(),
            Arc::clone(&attr_cache),
            Arc::clone(&dir_cache),
            Arc::clone(&read_cache),
        );

        let inodes = RwLock::new(InodeTable::new(root_path, cache.attr_capacity));
        let write_buffers = Mutex::new(WriteBuffers::new());

        Ok(Self {
            channel,
            inodes,
            attr_cache,
            dir_cache,
            read_cache,
            write_buffers,
            config,
            watch_handle,
        })
    }

    /// Looks up a child entry by name under the given parent inode.
    ///
    /// Returns the child's cached or freshly-fetched attributes. If the child
    /// is not yet in the inode table, a remote metadata call determines whether
    /// it exists and, if so, allocates an inode for it.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent inode is unknown or the remote metadata
    /// call fails (including when the child does not exist).
    pub async fn lookup(&self, parent_ino: u64, name: &str) -> io::Result<FileAttr> {
        let child_path = self.child_path(parent_ino, name).await?;

        debug!(
            "lookup parent={} name={:?} -> {}",
            parent_ino, name, child_path
        );

        // Check if inode already exists and has a cached attr.
        {
            let inodes = self.inodes.read().await;
            if let Some(ino) = inodes.get_ino(&child_path) {
                let mut attr_cache = self.attr_cache.lock().await;
                if let Some(cached) = attr_cache.get(&child_path) {
                    let mut attr = cached.attr.clone();
                    attr.ino = ino;

                    // Clear our locks so we can access inode for write
                    drop(attr_cache);
                    drop(inodes);

                    // Increment reference for this lookup.
                    let mut inodes_w = self.inodes.write().await;
                    inodes_w.inc_ref(ino);
                    return Ok(attr);
                }
            }
        }

        // Fetch metadata from remote.
        let metadata = self.fetch_metadata(&child_path).await?;
        let mut inodes = self.inodes.write().await;
        let ino = inodes.insert(child_path.clone());
        inodes.inc_ref(ino);

        let attr = FileAttr::from_metadata(ino, &metadata);
        let cached = CachedAttr {
            metadata,
            attr: attr.clone(),
        };
        let mut attr_cache = self.attr_cache.lock().await;
        attr_cache.insert(child_path, cached);

        Ok(attr)
    }

    /// Returns the file attributes for the given inode.
    ///
    /// Returns cached attributes when available, otherwise fetches fresh
    /// metadata from the remote server.
    ///
    /// # Errors
    ///
    /// Returns an error if the inode is unknown or the remote metadata call
    /// fails.
    pub async fn getattr(&self, ino: u64) -> io::Result<FileAttr> {
        let path = self.ino_to_path(ino).await?;

        debug!("getattr ino={} path={}", ino, path);

        // Check cache first.
        {
            let mut attr_cache = self.attr_cache.lock().await;
            if let Some(cached) = attr_cache.get(&path) {
                let mut attr = cached.attr.clone();
                attr.ino = ino;
                return Ok(attr);
            }
        }

        // Fetch from remote.
        let metadata = self.fetch_metadata(&path).await?;
        let attr = FileAttr::from_metadata(ino, &metadata);
        let cached = CachedAttr {
            metadata,
            attr: attr.clone(),
        };
        let mut attr_cache = self.attr_cache.lock().await;
        attr_cache.insert(path, cached);

        Ok(attr)
    }

    /// Returns the directory listing for the given inode.
    ///
    /// Returns cached entries when available, otherwise fetches a fresh listing
    /// from the remote server. Each entry in the listing is assigned an inode
    /// via the inode table.
    ///
    /// # Errors
    ///
    /// Returns an error if the inode is unknown or the remote `read_dir` call
    /// fails.
    pub async fn readdir(&self, ino: u64) -> io::Result<Vec<DirCacheEntry>> {
        let path = self.ino_to_path(ino).await?;

        debug!("readdir ino={} path={}", ino, path);

        // Check cache first.
        {
            let mut dir_cache = self.dir_cache.lock().await;
            if let Some(entries) = dir_cache.get(&path) {
                return Ok(entries.clone());
            }
        }

        // Fetch from remote. depth=1, absolute=true so paths are full.
        let mut ch = self.channel.clone();
        let (dir_entries, _errors) = ch.read_dir(path.clone(), 1, true, false, false).await?;

        // Convert DirEntry list to DirCacheEntry with inode allocation.
        let mut inodes = self.inodes.write().await;
        let mut entries = Vec::with_capacity(dir_entries.len());
        for entry in dir_entries {
            let entry_ino = inodes.insert(entry.path.clone());
            let name = extract_file_name(&entry.path);
            entries.push(DirCacheEntry {
                name,
                ino: entry_ino,
                file_type: entry.file_type,
            });
        }
        drop(inodes);

        let mut dir_cache = self.dir_cache.lock().await;
        dir_cache.insert(path, entries.clone());

        Ok(entries)
    }

    /// Reads file data for the given inode at the specified offset and size.
    ///
    /// The full file is cached on first access; subsequent reads within the
    /// cache TTL are served from memory. Returns the requested byte range,
    /// clamped to the actual file length.
    ///
    /// # Errors
    ///
    /// Returns an error if the inode is unknown or the remote `read_file` call
    /// fails.
    pub async fn read(&self, ino: u64, offset: u64, size: u32) -> io::Result<Vec<u8>> {
        let path = self.ino_to_path(ino).await?;

        debug!("read ino={} offset={} size={}", ino, offset, size);

        // Check cache first.
        {
            let mut read_cache = self.read_cache.lock().await;
            if let Some(data) = read_cache.get(&ino) {
                return Ok(slice_range(data, offset, size));
            }
        }

        // Fetch entire file from remote.
        let mut ch = self.channel.clone();
        let options = ReadFileOptions {
            offset: None,
            len: None,
        };
        let data = ch.read_file(path, options).await?;

        let result = slice_range(&data, offset, size);

        let mut read_cache = self.read_cache.lock().await;
        read_cache.insert(ino, data);

        Ok(result)
    }

    /// Buffers a write for the given inode.
    ///
    /// Data is accumulated in an in-memory [`WriteBuffer`](crate::write_buffer::WriteBuffer)
    /// and flushed to the remote on [`flush`](Self::flush), [`fsync`](Self::fsync),
    /// or [`release`](Self::release).
    ///
    /// # Errors
    ///
    /// Returns an error if the inode is unknown or a lock is poisoned.
    pub async fn write(&self, ino: u64, offset: u64, data: &[u8]) -> io::Result<u32> {
        let path = self.ino_to_path(ino).await?;

        debug!("write ino={} offset={} len={}", ino, offset, data.len());

        // Get original file size from attr cache for gap-filling.
        let original_size = {
            let mut attr_cache = self.attr_cache.lock().await;
            attr_cache
                .get(&path)
                .map(|cached| cached.attr.size)
                .unwrap_or(0)
        };

        let mut write_buffers = self.write_buffers.lock().await;
        let buf = write_buffers.get_or_create(ino, original_size);
        buf.write(offset, data);

        // Invalidate read cache since the file content is being modified.
        let mut read_cache = self.read_cache.lock().await;
        read_cache.invalidate(&ino);

        Ok(data.len() as u32)
    }

    /// Creates an empty file on the remote server.
    ///
    /// Writes zero bytes to create the file, then fetches its metadata to
    /// populate the inode table and attribute cache. The parent directory
    /// cache is invalidated.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent inode is unknown, the remote write fails,
    /// or the metadata fetch fails.
    pub async fn create(&self, parent_ino: u64, name: &str, _mode: u32) -> io::Result<FileAttr> {
        let child_path = self.child_path(parent_ino, name).await?;

        debug!(
            "create parent={} name={:?} -> {}",
            parent_ino, name, child_path
        );

        // Create empty file on remote.
        let mut ch = self.channel.clone();
        let options = WriteFileOptions {
            offset: None,
            append: false,
        };
        ch.write_file(child_path.clone(), Vec::new(), options)
            .await?;

        // Fetch metadata for the new file.
        let metadata = self.fetch_metadata(&child_path).await?;

        let mut inodes = self.inodes.write().await;
        let ino = inodes.insert(child_path.clone());
        inodes.inc_ref(ino);

        let attr = FileAttr::from_metadata(ino, &metadata);
        let cached = CachedAttr {
            metadata,
            attr: attr.clone(),
        };
        let mut attr_cache = self.attr_cache.lock().await;
        attr_cache.insert(child_path, cached);
        drop(attr_cache);

        // Invalidate parent dir cache.
        let parent_path = inodes.get_path(parent_ino);
        drop(inodes);
        if let Some(pp) = parent_path {
            let mut dir_cache = self.dir_cache.lock().await;
            dir_cache.invalidate(&pp);
        }

        Ok(attr)
    }

    /// Creates a directory on the remote server.
    ///
    /// Uses `create_dir` with `all=false` (single directory), then fetches its
    /// metadata to populate the inode table and attribute cache. The parent
    /// directory cache is invalidated.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent inode is unknown, the remote
    /// `create_dir` call fails, or the metadata fetch fails.
    pub async fn mkdir(&self, parent_ino: u64, name: &str, _mode: u32) -> io::Result<FileAttr> {
        let child_path = self.child_path(parent_ino, name).await?;

        debug!(
            "mkdir parent={} name={:?} -> {}",
            parent_ino, name, child_path
        );

        let mut ch = self.channel.clone();
        ch.create_dir(child_path.clone(), false).await?;

        // Fetch metadata for the new directory.
        let metadata = self.fetch_metadata(&child_path).await?;

        let mut inodes = self.inodes.write().await;
        let ino = inodes.insert(child_path.clone());
        inodes.inc_ref(ino);

        let attr = FileAttr::from_metadata(ino, &metadata);
        let cached = CachedAttr {
            metadata,
            attr: attr.clone(),
        };
        let mut attr_cache = self.attr_cache.lock().await;
        attr_cache.insert(child_path, cached);
        drop(attr_cache);

        // Invalidate parent dir cache.
        let parent_path = inodes.get_path(parent_ino);
        drop(inodes);
        if let Some(pp) = parent_path {
            let mut dir_cache = self.dir_cache.lock().await;
            dir_cache.invalidate(&pp);
        }

        Ok(attr)
    }

    /// Removes a file on the remote server.
    ///
    /// Invalidates the attribute and directory caches for both the file and
    /// its parent directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent inode is unknown or the remote `remove`
    /// call fails.
    pub async fn unlink(&self, parent_ino: u64, name: &str) -> io::Result<()> {
        let child_path = self.child_path(parent_ino, name).await?;

        debug!(
            "unlink parent={} name={:?} -> {}",
            parent_ino, name, child_path
        );

        let mut ch = self.channel.clone();
        ch.remove(child_path.clone(), false).await?;

        // Invalidate caches.
        let mut attr_cache = self.attr_cache.lock().await;
        attr_cache.invalidate(&child_path);
        drop(attr_cache);

        // Invalidate read cache if we have an inode for this file.
        {
            let inodes = self.inodes.read().await;
            if let Some(ino) = inodes.get_ino(&child_path) {
                let mut read_cache = self.read_cache.lock().await;
                read_cache.invalidate(&ino);
            }
        }

        let parent_path = self.ino_to_path(parent_ino).await?;
        let mut dir_cache = self.dir_cache.lock().await;
        dir_cache.invalidate(&parent_path);

        Ok(())
    }

    /// Removes a directory on the remote server.
    ///
    /// Invalidates the attribute and directory caches for both the directory
    /// and its parent.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent inode is unknown or the remote `remove`
    /// call fails.
    pub async fn rmdir(&self, parent_ino: u64, name: &str) -> io::Result<()> {
        let child_path = self.child_path(parent_ino, name).await?;

        debug!(
            "rmdir parent={} name={:?} -> {}",
            parent_ino, name, child_path
        );

        let mut ch = self.channel.clone();
        ch.remove(child_path.clone(), false).await?;

        // Invalidate caches.
        let mut attr_cache = self.attr_cache.lock().await;
        attr_cache.invalidate(&child_path);
        drop(attr_cache);

        let mut dir_cache = self.dir_cache.lock().await;
        dir_cache.invalidate(&child_path);
        drop(dir_cache);

        let parent_path = self.ino_to_path(parent_ino).await?;
        let mut dir_cache = self.dir_cache.lock().await;
        dir_cache.invalidate(&parent_path);

        Ok(())
    }

    /// Renames a file or directory on the remote server.
    ///
    /// Updates the inode table to reflect the new path and invalidates all
    /// affected caches (source attr, dest attr, both parent dir caches).
    ///
    /// # Errors
    ///
    /// Returns an error if either parent inode is unknown or the remote
    /// `rename` call fails.
    pub async fn rename(
        &self,
        parent_ino: u64,
        name: &str,
        new_parent_ino: u64,
        new_name: &str,
    ) -> io::Result<()> {
        let old_path = self.child_path(parent_ino, name).await?;
        let new_path = self.child_path(new_parent_ino, new_name).await?;

        debug!("rename {} -> {}", old_path, new_path);

        let mut ch = self.channel.clone();
        ch.rename(old_path.clone(), new_path.clone()).await?;

        // Update inode table.
        let mut inodes = self.inodes.write().await;
        if let Some(ino) = inodes.get_ino(&old_path) {
            inodes.rename(ino, new_path.clone());
        }
        let old_parent_path = inodes.get_path(parent_ino);
        let new_parent_path = inodes.get_path(new_parent_ino);
        drop(inodes);

        // Invalidate attr caches for old and new paths.
        let mut attr_cache = self.attr_cache.lock().await;
        attr_cache.invalidate(&old_path);
        attr_cache.invalidate(&new_path);
        drop(attr_cache);

        // Invalidate dir caches for both parents.
        let mut dir_cache = self.dir_cache.lock().await;
        if let Some(pp) = old_parent_path {
            dir_cache.invalidate(&pp);
        }
        if let Some(pp) = new_parent_path {
            dir_cache.invalidate(&pp);
        }

        Ok(())
    }

    /// Flushes any buffered writes for the given inode to the remote server.
    ///
    /// If the write buffer is dirty, the full buffer contents are written to
    /// the remote file and the buffer is cleared. The attribute and read caches
    /// are invalidated to reflect the updated content.
    ///
    /// # Errors
    ///
    /// Returns an error if the inode is unknown or the remote `write_file`
    /// call fails.
    pub async fn flush(&self, ino: u64) -> io::Result<()> {
        debug!("flush ino={}", ino);

        let mut write_buffers = self.write_buffers.lock().await;

        // Check if there is a dirty buffer to flush.
        let needs_flush = write_buffers.get(ino).is_some_and(|buf| buf.is_dirty());

        if !needs_flush {
            return Ok(());
        }

        let path = self.ino_to_path(ino).await?;

        // Safety: we just checked that the buffer exists and is dirty above,
        // and we still hold the lock.
        let buf = write_buffers
            .get_mut(ino)
            .expect("buffer exists (checked above)");
        let data = buf.data().to_vec();

        let mut ch = self.channel.clone();
        let options = WriteFileOptions {
            offset: None,
            append: false,
        };
        ch.write_file(path.clone(), data, options).await?;

        buf.clear();
        drop(write_buffers);

        // Invalidate caches since the file content changed.
        let mut attr_cache = self.attr_cache.lock().await;
        attr_cache.invalidate(&path);
        drop(attr_cache);

        let mut read_cache = self.read_cache.lock().await;
        read_cache.invalidate(&ino);

        Ok(())
    }

    /// Synchronizes buffered writes to the remote server.
    ///
    /// Equivalent to [`flush`](Self::flush).
    ///
    /// # Errors
    ///
    /// Returns an error if the flush fails.
    pub async fn fsync(&self, ino: u64) -> io::Result<()> {
        self.flush(ino).await
    }

    /// Flushes and releases the write buffer for the given inode.
    ///
    /// Called when a file handle is closed. Any dirty data is flushed first,
    /// then the write buffer is removed entirely.
    ///
    /// # Errors
    ///
    /// Returns an error if the flush fails.
    pub async fn release(&self, ino: u64) -> io::Result<()> {
        debug!("release ino={}", ino);

        self.flush(ino).await?;

        let mut write_buffers = self.write_buffers.lock().await;
        write_buffers.remove(ino);

        Ok(())
    }

    /// Decrements the reference count for the given inode.
    ///
    /// Called by the kernel to indicate that `nlookup` references to this
    /// inode have been released. When the reference count reaches zero the
    /// inode becomes eligible for eviction from the inode table.
    pub async fn forget(&self, ino: u64, nlookup: u64) {
        debug!("forget ino={} nlookup={}", ino, nlookup);

        let mut inodes = self.inodes.write().await;
        inodes.dec_ref(ino, nlookup);
    }

    /// Returns the remote path associated with the given inode, if present.
    ///
    /// Unlike [`ino_to_path`](Self::ino_to_path), this returns `None` instead
    /// of an error when the inode is unknown or the lock is poisoned.
    #[allow(dead_code)]
    pub async fn get_path(&self, ino: u64) -> Option<RemotePath> {
        let inodes = self.inodes.read().await;
        inodes.get_path(ino)
    }

    /// Returns the inode number associated with the given remote path, if
    /// present.
    #[allow(dead_code)]
    pub async fn get_ino_for_path(&self, path: &str) -> Option<u64> {
        let inodes = self.inodes.read().await;
        inodes.get_ino(&RemotePath::new(path))
    }

    /// Builds a child path by joining the parent inode's path with a name.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent inode is not in the inode table.
    async fn child_path(&self, parent_ino: u64, name: &str) -> io::Result<RemotePath> {
        let parent_path = self.ino_to_path(parent_ino).await?;
        let parent_str = parent_path.as_str();
        let child = if parent_str.ends_with('/') {
            RemotePath::new(format!("{parent_str}{name}"))
        } else {
            RemotePath::new(format!("{parent_str}/{name}"))
        };
        Ok(child)
    }

    /// Resolves an inode number to its remote path.
    ///
    /// # Errors
    ///
    /// Returns `ENOENT` if the inode is not in the table.
    async fn ino_to_path(&self, ino: u64) -> io::Result<RemotePath> {
        let inodes = self.inodes.read().await;
        inodes
            .get_path(ino)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("unknown inode {ino}")))
    }

    /// Fetches metadata from the remote server for the given path.
    async fn fetch_metadata(&self, path: &RemotePath) -> io::Result<Metadata> {
        let mut ch = self.channel.clone();
        ch.metadata(path.clone(), false, true).await
    }
}

impl Drop for RemoteFs {
    fn drop(&mut self) {
        if let Some(handle) = self.watch_handle.take() {
            handle.abort();
        }
    }
}

/// Spawns a best-effort watch task that invalidates caches when remote
/// filesystem changes are detected.
///
/// If the plugin does not support watching (e.g. Docker, SSH), logs a
/// warning and returns `None`.
fn spawn_watch_task(
    channel: Channel,
    remote_root: RemotePath,
    attr_cache: Arc<Mutex<AttrCache>>,
    dir_cache: Arc<Mutex<DirCache>>,
    read_cache: Arc<Mutex<ReadCache>>,
) -> Option<tokio::task::JoinHandle<()>> {
    let mut watch_channel = channel.clone();

    let handle = tokio::spawn(async move {
        match watch_channel
            .watch(
                remote_root,
                true,
                Vec::<ChangeKind>::new(),
                Vec::<ChangeKind>::new(),
            )
            .await
        {
            Ok(mut watcher) => {
                debug!("watch-based cache invalidation active");
                while let Some(change) = watcher.next().await {
                    invalidate_for_change(&attr_cache, &dir_cache, &read_cache, &change).await;
                }
                debug!("watcher stream ended");
            }
            Err(e) => {
                warn!(
                    "watch not available for this connection, cache invalidation \
                     will rely on TTL only: {e}"
                );
            }
        }
    });

    Some(handle)
}

/// Invalidates the appropriate caches based on a filesystem change event.
async fn invalidate_for_change(
    attr_cache: &Arc<Mutex<AttrCache>>,
    dir_cache: &Arc<Mutex<DirCache>>,
    read_cache: &Arc<Mutex<ReadCache>>,
    change: &Change,
) {
    let path = &change.path;

    debug!("cache invalidation for {:?} on {}", change.kind, path);

    match change.kind {
        ChangeKind::Create | ChangeKind::Delete | ChangeKind::Rename => {
            attr_cache.lock().await.invalidate(path);

            // Invalidate parent directory listing.
            let parent = parent_path(path);
            let mut cache = dir_cache.lock().await;
            cache.invalidate(&parent);
            cache.invalidate(path);
        }
        ChangeKind::Modify | ChangeKind::CloseWrite => {
            attr_cache.lock().await.invalidate(path);

            // Read cache is keyed by inode, which we don't have here.
            // Clear the entire read cache as a conservative approach.
            read_cache.lock().await.clear();
        }
        _ => {}
    }
}

/// Returns the parent path of a remote path.
fn parent_path(path: &RemotePath) -> RemotePath {
    let s = path.as_str();

    // TODO: This won't work on Windows
    match s.rsplit_once('/') {
        Some(("", _)) => RemotePath::new("/"),
        Some((parent, _)) => RemotePath::new(parent),
        None => RemotePath::new("/"),
    }
}

/// Extracts the final path component from a remote path string.
///
/// Falls back to the full path string if no `/` separator is found.
fn extract_file_name(path: &RemotePath) -> String {
    let s = path.as_str();

    // TODO: This won't work on Windows
    match s.rfind('/') {
        Some(pos) => s[pos + 1..].to_string(),
        None => s.to_string(),
    }
}

/// Extracts a byte range from a data slice, clamping to the actual length.
fn slice_range(data: &[u8], offset: u64, size: u32) -> Vec<u8> {
    let start = (offset as usize).min(data.len());
    let end = (start + size as usize).min(data.len());
    data[start..end].to_vec()
}
