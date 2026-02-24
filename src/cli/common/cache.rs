use std::io;
use std::path::{Path, PathBuf};

use anyhow::Context;
use distant_core::net::common::ConnectionId;
use serde::{Deserialize, Serialize};

use crate::constants::user::CACHE_FILE_PATH;

mod id;
pub use id::CacheId;

/// Represents a disk-backed cache of data
#[derive(Clone, Debug)]
pub struct Cache {
    file: CacheFile,
    pub data: CacheData,
}

impl Cache {
    pub fn new(custom_path: impl Into<Option<PathBuf>>) -> Self {
        Self {
            file: CacheFile::new(custom_path),
            data: CacheData::default(),
        }
    }

    /// Loads the cache from the specified file path, or default user-local cache path,
    /// constructing data from the default cache if not found
    pub async fn read_from_disk_or_default(
        custom_path: impl Into<Option<PathBuf>>,
    ) -> anyhow::Result<Self> {
        let file = CacheFile::new(custom_path);
        let data = file.read_or_default().await?;
        Ok(Self { file, data })
    }

    /// Writes the cache back to disk
    pub async fn write_to_disk(&self) -> anyhow::Result<()> {
        self.file.write(&self.data).await
    }
}

/// Points to a cache file to support reading, writing, and editing the data
#[derive(Clone, Debug)]
pub struct CacheFile {
    path: PathBuf,
}

impl CacheFile {
    /// Creates a new [`CacheFile`] from the given path, defaulting to a user-local cache path
    /// if none is provided
    pub fn new(custom_path: impl Into<Option<PathBuf>>) -> Self {
        Self {
            path: custom_path
                .into()
                .unwrap_or_else(|| CACHE_FILE_PATH.to_path_buf()),
        }
    }

    async fn read_or_default(&self) -> anyhow::Result<CacheData> {
        CacheData::read_or_default(self.path.as_path())
            .await
            .with_context(|| format!("Failed to read cache from {:?}", self.path.as_path()))
    }

    async fn write(&self, data: &CacheData) -> anyhow::Result<()> {
        data.write(self.path.as_path())
            .await
            .with_context(|| format!("Failed to write cache to {:?}", self.path.as_path()))
    }
}

/// Provides quick access to cli-specific cache for a user
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CacheData {
    /// Connection id of selected connection (or 0 if nothing selected)
    pub selected: CacheId<ConnectionId>,
}

impl CacheData {
    /// Reads the cache data from disk
    async fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = tokio::fs::read(path).await?;
        toml_edit::de::from_slice(&bytes).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Reads the cache data if the file exists, otherwise returning a default cache instance
    async fn read_or_default(path: impl AsRef<Path>) -> io::Result<Self> {
        match Self::read(path).await {
            Ok(cache) => Ok(cache),
            Err(x) if x.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(x) => Err(x),
        }
    }

    /// Writes the cache data to disk
    async fn write(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let bytes = toml_edit::ser::to_vec(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Ensure the parent directory of the cache exists
        if let Some(parent) = path.as_ref().parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(path, bytes).await
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `Cache`, `CacheFile`, and `CacheData`: construction, defaults,
    //! serialization, disk I/O round-trips, and error handling.

    use super::*;

    #[test]
    fn cache_new_with_custom_path() {
        let cache = Cache::new(PathBuf::from("/tmp/test-cache.toml"));
        assert_eq!(*cache.data.selected, 0);
    }

    #[test]
    fn cache_new_with_none_uses_default() {
        let cache = Cache::new(None::<PathBuf>);
        assert_eq!(*cache.data.selected, 0);
    }

    #[test]
    fn cache_file_new_with_custom_path() {
        let cf = CacheFile::new(PathBuf::from("/tmp/test.toml"));
        assert_eq!(cf.path, PathBuf::from("/tmp/test.toml"));
    }

    #[test]
    fn cache_file_new_with_none_uses_default() {
        let cf = CacheFile::new(None::<PathBuf>);
        assert_eq!(cf.path, CACHE_FILE_PATH.to_path_buf());
    }

    #[test]
    fn cache_data_default_has_zero_selected() {
        let data = CacheData::default();
        assert_eq!(*data.selected, 0);
    }

    #[test]
    fn cache_data_serialize_deserialize_round_trip() {
        let data = CacheData::default();
        let bytes = toml_edit::ser::to_vec(&data).unwrap();
        let restored: CacheData = toml_edit::de::from_slice(&bytes).unwrap();
        assert_eq!(*restored.selected, 0);
    }

    #[test]
    fn cache_clone() {
        let cache = Cache::new(PathBuf::from("/tmp/clone-test.toml"));
        let cloned = cache.clone();
        assert_eq!(*cloned.data.selected, *cache.data.selected);
    }

    #[tokio::test]
    async fn cache_write_and_read_round_trip() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("cache.toml");

        let cache = Cache::new(path.clone());
        cache.write_to_disk().await.unwrap();

        let loaded = Cache::read_from_disk_or_default(path).await.unwrap();
        assert_eq!(*loaded.data.selected, 0);
    }

    #[tokio::test]
    async fn cache_read_from_nonexistent_returns_default() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.toml");

        let loaded = Cache::read_from_disk_or_default(path).await.unwrap();
        assert_eq!(*loaded.data.selected, 0);
    }

    #[tokio::test]
    async fn cache_data_read_invalid_toml_returns_error() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("bad.toml");
        tokio::fs::write(&path, b"{{invalid toml}}").await.unwrap();

        let result = CacheData::read(&path).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn cache_data_write_creates_parent_directories() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("sub").join("dir").join("cache.toml");

        let data = CacheData::default();
        data.write(&path).await.unwrap();

        assert!(path.exists());
    }
}
