use crate::paths::user::STORAGE_FILE_PATH;
use distant_core::ConnectionId;
use serde::{Deserialize, Serialize};
use std::io;

mod id;
pub use id::StorageId;

/// Provides quick access to cli-specific storage for a user
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Storage {
    pub default_connection_id: StorageId<ConnectionId>,
}

impl Storage {
    /// Reads the storage data from disk
    pub async fn read() -> io::Result<Self> {
        let bytes = tokio::fs::read(STORAGE_FILE_PATH.as_path()).await?;
        toml_edit::de::from_slice(&bytes).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Reads the storage data if the file exists, otherwise returning a default storage instance
    pub async fn read_or_default() -> io::Result<Self> {
        match Self::read().await {
            Ok(storage) => Ok(storage),
            Err(x) if x.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(x) => Err(x),
        }
    }

    /// Writes the storage data to disk
    pub async fn write(&self) -> io::Result<()> {
        let bytes = toml_edit::ser::to_vec(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        tokio::fs::write(STORAGE_FILE_PATH.as_path(), bytes).await
    }
}
