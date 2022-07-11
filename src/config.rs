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

    /// Blocking version of `load_from_file`
    pub fn blocking_load_from_file(path: &Path) -> io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        toml_edit::de::from_str(&text).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Saves the configuration to the specified file
    pub async fn save_to_file(&self, path: &Path) -> io::Result<()> {
        let text = toml_edit::ser::to_string_pretty(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        tokio::fs::write(path, text).await
    }
}
