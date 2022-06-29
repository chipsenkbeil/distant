use serde::{Deserialize, Serialize};
use std::{io, path::Path};

mod client;
mod common;
mod manager;
mod network;
mod server;
mod service;

pub use client::*;
pub use common::*;
pub use manager::*;
pub use network::*;
pub use server::*;
pub use service::*;

/// Interface to merge one object into another
pub trait Merge<Rhs = Self> {
    /// Merges the right-hand side into the left-hand side
    fn merge(&mut self, other: Rhs);
}

/// Represents configuration settings for all of distant
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    pub client: ClientConfig,
    pub manager: ManagerConfig,
    pub server: ServerConfig,
}

impl Config {
    /// Loads the configuration from the specified file, defaulting to the standard config file
    pub async fn load_from_file(path: &Path) -> io::Result<Self> {
        let text = tokio::fs::read_to_string(path).await?;
        toml_edit::de::from_str(&text).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Saves the configuration to the specified file, defaulting to the standard config file
    pub async fn save_to_file(&self, path: &Path) -> io::Result<()> {
        let text = toml_edit::ser::to_string_pretty(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        tokio::fs::write(path, text).await
    }
}

impl Merge for Config {
    fn merge(&mut self, other: Self) {
        self.client.merge(other.client);
        self.manager.merge(other.manager);
        self.server.merge(other.server);
    }
}

impl Merge<ClientConfig> for Config {
    fn merge(&mut self, other: ClientConfig) {
        self.client.merge(other);
    }
}

impl Merge<ManagerConfig> for Config {
    fn merge(&mut self, other: ManagerConfig) {
        self.manager.merge(other);
    }
}

impl Merge<ServerConfig> for Config {
    fn merge(&mut self, other: ServerConfig) {
        self.server.merge(other);
    }
}

impl Merge<CommonConfig> for Config {
    fn merge(&mut self, other: CommonConfig) {
        self.client.merge(other.clone());
        self.manager.merge(other.clone());
        self.server.merge(other);
    }
}

impl Merge<NetworkConfig> for Config {
    fn merge(&mut self, other: NetworkConfig) {
        self.client.merge(other.clone());
        self.manager.merge(other);
    }
}
