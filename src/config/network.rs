use clap::Args;
use serde::{Deserialize, Serialize};

/// Represents common networking configuration
#[derive(Args, Clone, Debug, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Path to Unix socket
    #[cfg(unix)]
    #[clap(long)]
    pub unix_socket: Option<std::path::PathBuf>,

    /// Name of local Windows pipe
    #[cfg(windows)]
    #[clap(long)]
    pub windows_pipe: Option<String>,
}

impl NetworkConfig {
    pub fn merge(self, other: Self) -> Self {
        Self {
            #[cfg(unix)]
            unix_socket: self.unix_socket.or(other.unix_socket),

            #[cfg(windows)]
            windows_pipe: self.windows_pipe.or(other.windows_pipe),
        }
    }

    /// Returns a collection of candidate unix socket paths, which will either be
    /// the config-provided unix socket path or the default user and global socket paths
    #[cfg(unix)]
    pub fn to_unix_socket_path_candidates(&self) -> Vec<&std::path::Path> {
        match self.unix_socket.as_deref() {
            Some(path) => vec![path],
            None => vec![
                crate::paths::user::UNIX_SOCKET_PATH.as_path(),
                crate::paths::global::UNIX_SOCKET_PATH.as_path(),
            ],
        }
    }

    /// Returns a collection of candidate windows pipe names, which will either be
    /// the config-provided windows pipe name or the default user and global pipe names
    #[cfg(windows)]
    pub fn to_windows_pipe_name_candidates(&self) -> Vec<&str> {
        match self.windows_pipe.as_deref() {
            Some(name) => vec![name],
            None => vec![
                crate::paths::user::WINDOWS_PIPE_NAME.as_str(),
                crate::paths::global::WINDOWS_PIPE_NAME.as_str(),
            ],
        }
    }
}
