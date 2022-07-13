use distant_core::ConnectionId;
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

mod id;
pub use id::StorageId;

/// Provides quick access to cli-specific storage for a user
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Storage {
    pub default_connection_id: StorageId<ConnectionId>,
}

impl Storage {
    /// Loads (or creates) [`Storage`], modifies it, and writes it back to `path`
    pub async fn edit(path: impl AsRef<Path>, f: impl FnOnce(&mut Self)) -> io::Result<()> {
        let mut this = Self::read_or_default(path.as_ref()).await?;
        f(&mut this);
        this.write(path).await
    }

    /// Reads the storage data from disk
    pub async fn read(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = tokio::fs::read(path).await?;
        toml_edit::de::from_slice(&bytes).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Reads the storage data if the file exists, otherwise returning a default storage instance
    pub async fn read_or_default(path: impl AsRef<Path>) -> io::Result<Self> {
        match Self::read(path).await {
            Ok(storage) => Ok(storage),
            Err(x) if x.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(x) => Err(x),
        }
    }

    /// Writes the storage data to disk
    pub async fn write(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let bytes = toml_edit::ser::to_vec(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        tokio::fs::write(path, bytes).await
    }
}
