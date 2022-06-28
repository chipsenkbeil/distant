use directories::ProjectDirs;
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};

/// Root project directory used to calculate other paths
static PROJECT_DIR: Lazy<ProjectDirs> = Lazy::new(|| {
    ProjectDirs::from("", "", "distant").expect("Could not determine valid $HOME path")
});

/// Path to configuration settings for distant client/manager/server
pub static CONFIG_FILE_PATH: Lazy<PathBuf> =
    Lazy::new(|| PROJECT_DIR.config_dir().join("config.toml"));

/// For Linux, this uses the runtime path. For Mac, this uses the tmp path
///
/// * `/run/user/1001/distant/distant.sock`
/// * `/tmp/distant.dock`
#[cfg(unix)]
pub static UNIX_SOCKET_PATH: Lazy<PathBuf> = Lazy::new(|| {
    PROJECT_DIR
        .runtime_dir()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
        .join("distant.sock")
});

/// Name of the pipe used by Windows
#[cfg(windows)]
pub const WINDOWS_PIPE_NAME: &str = "distant";

/// Name of user executing the cli
pub static USERNAME: Lazy<String> = Lazy::new(whoami::username);
