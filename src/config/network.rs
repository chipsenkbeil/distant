use crate::Merge;
use clap::Args;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Represents common networking configuration
#[derive(Args, Clone, Debug, Default, Serialize, Deserialize)]
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

impl Merge for NetworkConfig {
    fn merge(&mut self, other: Self) {
        #[cfg(unix)]
        if let Some(x) = other.unix_socket {
            self.unix_socket = Some(x);
        }

        #[cfg(windows)]
        if let Some(x) = other.windows_pipe {
            self.windows_pipe = Some(x);
        }
    }
}