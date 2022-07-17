use clap::Args;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;

/// Represents common networking configuration
#[derive(Args, Clone, Debug, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// If specified, will default to user-local socket or pipe
    #[clap(long)]
    pub user: bool,

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
            user: self.user || other.user,

            #[cfg(unix)]
            unix_socket: self.unix_socket.or(other.unix_socket),

            #[cfg(windows)]
            windows_pipe: self.windows_pipe.or(other.windows_pipe),
        }
    }

    /// Creates a string describing the active method (Unix Socket or Windows Pipe)
    pub fn to_method_string(&self) -> String {
        #[cfg(unix)]
        {
            format!("<Unix Socket {:?}>", self.unix_socket_path_or_default())
        }
        #[cfg(windows)]
        {
            format!("<Windows Pipe {:?}>", self.windows_pipe_name_or_default())
        }
    }

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
