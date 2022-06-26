use crate::constants::CONFIG_FILE_PATH;
use serde::{Deserialize, Serialize};
use std::{io, path::Path};

mod client;
mod manager;
mod server;

pub use client::*;
pub use manager::*;
pub use server::*;

/// Represents configuration settings for all of distant
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub client: ClientConfig,
    pub manager: ManagerConfig,
    pub server: ServerConfig,
}

impl Config {
    /// Loads the configuration from the specified file, defaulting to the standard config file
    pub async fn load_from_file(path: Option<&Path>) -> io::Result<Self> {
        let text = tokio::fs::read_to_string(path.unwrap_or(CONFIG_FILE_PATH.as_path())).await?;
        toml::from_str(&text).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Saves the configuration to the specified file, defaulting to the standard config file
    pub async fn save_to_file(&self, path: Option<&Path>) -> io::Result<()> {
        let text = toml::to_string_pretty(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        tokio::fs::write(path.unwrap_or(CONFIG_FILE_PATH.as_path()), text).await
    }
}
