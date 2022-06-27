use clap::Args;
use merge::Merge;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

mod client;
mod common;
mod manager;
mod server;

pub use client::*;
pub use common::*;
pub use manager::*;
pub use server::*;

/// Represents configuration settings for all of distant
#[derive(Debug, Default, Merge, Serialize, Deserialize)]
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

/// Represents common networking configuration
#[derive(Args, Debug, Default, Merge, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Unix socket to listen on
    #[cfg(unix)]
    #[clap(long)]
    pub unix_socket: Option<PathBuf>,

    /// Windows pipe to listen on
    #[cfg(windows)]
    #[clap(long)]
    pub windows_pipe: Option<String>,
}

impl NetworkConfig {
    /// Returns the custom unix socket path, or the default path
    #[cfg(unix)]
    pub fn unix_socket_path_or_default(&self) -> &Path {
        self.unix_socket
            .as_deref()
            .unwrap_or(crate::constants::UNIX_SOCKET_PATH.as_path())
    }

    /// Returns the custom windows pipe name, or the default name
    #[cfg(windows)]
    pub fn windows_pipe_name_or_default(&self) -> &str {
        self.windows_pipe
            .as_deref()
            .unwrap_or(crate::constants::WINDOWS_PIPE_NAME.as_str())
    }
}
