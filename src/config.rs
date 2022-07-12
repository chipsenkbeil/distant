use crate::paths;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};
use toml_edit::Document;

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
    /// Loads the configuration from multiple sources in a blocking fashion
    ///
    /// 1. If `custom` is provided, it is used by itself as the source for configuration
    /// 2. Otherwise, if `custom` is not provided, will attempt to load from global and user
    ///    config files, merging together if they both exist
    /// 3. Otherwise if no `custom` path and none of the standard configuration paths exist,
    ///    then the default configuration is returned instead
    pub fn load_multi(custom: Option<PathBuf>) -> io::Result<Self> {
        match custom {
            Some(path) => toml_edit::de::from_slice(&std::fs::read(path)?)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x)),
            None => {
                let paths = vec![
                    paths::global::CONFIG_FILE_PATH.as_path(),
                    paths::user::CONFIG_FILE_PATH.as_path(),
                ];

                match (paths[0].exists(), paths[1].exists()) {
                    // At least one standard path exists, so load it
                    (exists_1, exists_2) if exists_1 || exists_2 => {
                        use config::{Config, File};
                        let config = Config::builder()
                            .add_source(File::from(paths[0]).required(exists_1))
                            .add_source(File::from(paths[1]).required(exists_2))
                            .build()
                            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
                        config
                            .try_deserialize()
                            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
                    }

                    // None of our standard paths exist, so use the default value instead
                    _ => Ok(Self::default()),
                }
            }
        }
    }

    /// Loads the specified `path` as a [`Config`]
    pub async fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = tokio::fs::read(path).await?;
        toml_edit::de::from_slice(&bytes).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Like `edit` but will succeed without invoking `f` if the path is not found
    pub async fn edit_if_exists(
        path: impl AsRef<Path>,
        f: impl FnOnce(&mut Document) -> io::Result<()>,
    ) -> io::Result<()> {
        Self::edit(path, f).await.or_else(|x| {
            if x.kind() == io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(x)
            }
        })
    }

    /// Loads the specified `path` as a [`Document`], performs changes to the document using `f`,
    /// and overwrites the `path` with the updated [`Document`]
    pub async fn edit(
        path: impl AsRef<Path>,
        f: impl FnOnce(&mut Document) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut document = tokio::fs::read_to_string(path.as_ref())
            .await?
            .parse::<Document>()
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        f(&mut document)?;
        tokio::fs::write(path, document.to_string()).await
    }

    /// Saves the [`Config`] to the specified `path` only if the path points to no file
    pub async fn save_if_not_found(&self, path: impl AsRef<Path>) -> io::Result<()> {
        use tokio::io::AsyncWriteExt;
        let text = toml_edit::ser::to_string_pretty(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        tokio::fs::OpenOptions::new()
            .create_new(true)
            .open(path)
            .await?
            .write_all(text.as_bytes())
            .await
    }

    /// Saves the [`Config`] to the specified `path`, overwriting the file if it exists
    pub async fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let text = toml_edit::ser::to_string_pretty(self)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        tokio::fs::write(path, text).await
    }
}
