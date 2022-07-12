use clap::Args;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;

/// Represents common networking configuration
#[derive(Args, Clone, Debug, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// If specified, will default to user-local socket or pipe
    #[clap(long)]
    pub user: bool,

    /// Unix socket to listen on
    #[cfg(unix)]
    #[clap(long)]
    pub unix_socket: Option<std::path::PathBuf>,

    /// Windows pipe to listen on
    #[cfg(windows)]
    #[clap(long)]
    pub windows_pipe: Option<String>,
}

impl NetworkConfig {
    /// Returns either the unix socket or windows pipe name as an [`OsStr`]
    pub fn as_os_str(&self) -> &OsStr {
        #[cfg(unix)]
        {
            self.unix_socket_path_or_default().as_os_str()
        }

        #[cfg(windows)]
        {
            self.windows_pipe_name_or_default().as_ref()
        }
    }

    /// Returns the custom unix socket path, or the default path
    #[cfg(unix)]
    pub fn unix_socket_path_or_default(&self) -> &std::path::Path {
        self.unix_socket.as_deref().unwrap_or(if self.user {
            crate::paths::user::UNIX_SOCKET_PATH.as_path()
        } else {
            crate::paths::global::UNIX_SOCKET_PATH.as_path()
        })
    }

    /// Returns the custom windows pipe name, or the default name
    #[cfg(windows)]
    pub fn windows_pipe_name_or_default(&self) -> &str {
        self.windows_pipe.as_deref().unwrap_or(if self.user {
            crate::paths::user::WINDOWS_PIPE_NAME.as_str()
        } else {
            crate::paths::global::WINDOWS_PIPE_NAME.as_str()
        })
    }
}
