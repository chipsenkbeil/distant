use crate::paths::user::CACHE_FILE_PATH;
use distant_core::ConnectionId;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

mod id;
pub use id::CacheId;

/// Represents a disk-backed cache of data
#[derive(Clone, Debug)]
pub struct Cache {
    file: CacheFile,
    pub data: CacheData,
}

impl Cache {
    /// Loads the cache from the specified file path, or default user-local cache path,
    /// constructing data from the default cache if not found
    pub async fn read_from_disk_or_default(
        custom_path: impl Into<Option<PathBuf>>,
    ) -> io::Result<Self> {
        let file = CacheFile::new(custom_path);
        let data = file.read_or_default().await?;
        Ok(Self { file, data })
    }

    /// Writes the cache back to disk
    pub async fn write_to_disk(&self) -> io::Result<()> {
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

    pub async fn read_or_default(&self) -> io::Result<CacheData> {
        CacheData::read_or_default(self.path.as_path()).await
    }

    pub async fn write(&self, data: &CacheData) -> io::Result<()> {
        data.write(self.path.as_path()).await
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
    pub async fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = tokio::fs::read(path).await?;
        toml_edit::de::from_slice(&bytes).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Reads the cache data if the file exists, otherwise returning a default cache instance
    pub async fn read_or_default(path: impl AsRef<Path>) -> io::Result<Self> {
        match Self::read(path).await {
            Ok(cache) => Ok(cache),
            Err(x) if x.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(x) => Err(x),
        }
    }

    /// Writes the cache data to disk
    pub async fn write(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let bytes = toml_edit::ser::to_vec(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        tokio::fs::write(path, bytes).await
    }
}
