use serde::{Deserialize, Serialize};
use std::{io, path::Path};

mod client;
mod common;
mod manager;
mod network;
mod server;

pub use client::*;
pub use common::*;
pub use manager::*;
pub use network::*;
pub use server::*;

/// Represents configuration settings for all of distant
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub client: ClientConfig,
    pub manager: ManagerConfig,
    pub server: ServerConfig,
}

impl Config {
    /// Loads the configuration from the specified file
    pub async fn load_from_file(path: &Path) -> io::Result<Self> {
        let text = tokio::fs::read_to_string(path).await?;
        toml_edit::de::from_str(&text).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Loads the configuration from the specified file
    pub fn blocking_load_from_file(path: &Path) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        toml_edit::de::from_str(&text).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Loads the configuration from the specified paths, ovewriting duplication
    /// configuration settings in the order of `1 -> 2 -> 3`
    pub fn load_from_files<'a>(paths: impl Iterator<Item = &'a Path>) -> io::Result<Self> {
        use config::{Config, File};
        let config = Config::builder()
            .add_source(paths.map(File::from).collect::<Vec<_>>())
            .build()
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
        config
            .try_deserialize()
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
    }

    /// Saves the configuration to the specified file
    pub async fn save_to_file(&self, path: &Path) -> io::Result<()> {
        let text = toml_edit::ser::to_string_pretty(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        tokio::fs::write(path, text).await
    }
}
