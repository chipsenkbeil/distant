use directories::ProjectDirs;
use once_cell::sync::Lazy;
use std::path::PathBuf;

/// Represents the maximum size (in bytes) that data will be read from pipes
/// per individual `read` call
///
/// Current setting is 16k size
pub const MAX_PIPE_CHUNK_SIZE: usize = 16384;

/// Internal name to use for socket files.
const SOCKET_FILE_STR: &str = "distant.sock";

/// User-oriented paths.
pub mod user {
    use super::*;

    /// Root project directory used to calculate other paths
    static PROJECT_DIR: Lazy<ProjectDirs> = Lazy::new(|| {
        ProjectDirs::from("", "", "distant").expect("Could not determine valid $HOME path")
    });

    /// Path to configuration settings for distant client/manager/server
    pub static CONFIG_FILE_PATH: Lazy<PathBuf> =
        Lazy::new(|| PROJECT_DIR.config_dir().join("config.toml"));

    /// Path to cache file used for arbitrary CLI data
    pub static CACHE_FILE_PATH: Lazy<PathBuf> =
        Lazy::new(|| PROJECT_DIR.cache_dir().join("cache.toml"));

    pub static CACHE_FILE_PATH_STR: Lazy<String> =
        Lazy::new(|| CACHE_FILE_PATH.to_string_lossy().to_string());

    /// Path to log file for distant client
    pub static CLIENT_LOG_FILE_PATH: Lazy<PathBuf> =
        Lazy::new(|| PROJECT_DIR.cache_dir().join("client.log"));

    /// Path to log file for distant manager
    pub static MANAGER_LOG_FILE_PATH: Lazy<PathBuf> =
        Lazy::new(|| PROJECT_DIR.cache_dir().join("manager.log"));

    /// Path to log file for distant server
    pub static SERVER_LOG_FILE_PATH: Lazy<PathBuf> =
        Lazy::new(|| PROJECT_DIR.cache_dir().join("server.log"));

    /// Path to log file for distant generate
    pub static GENERATE_LOG_FILE_PATH: Lazy<PathBuf> =
        Lazy::new(|| PROJECT_DIR.cache_dir().join("generate.log"));

    /// For Linux & BSD, this uses the runtime path. For Mac, this uses the tmp path
    ///
    /// * `/run/user/1001/distant/{user}.distant.sock` on Linux
    /// * `/var/run/{user}.distant.sock` on BSD
    /// * `/tmp/{user}.distant.dock` on MacOS
    pub static UNIX_SOCKET_PATH: Lazy<PathBuf> = Lazy::new(|| {
        // Form of {user}.distant.sock
        let mut file_name = whoami::username_os();
        file_name.push(".");
        file_name.push(SOCKET_FILE_STR);

        PROJECT_DIR
            .runtime_dir()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir)
            .join(file_name)
    });

    /// Name of the pipe used by Windows in the form of `{user}.distant`
    pub static WINDOWS_PIPE_NAME: Lazy<String> =
        Lazy::new(|| format!("{}.distant", whoami::username()));
}

/// Global paths.
pub mod global {
    use super::*;

    /// Windows ProgramData directory from from the %ProgramData% environment variable
    #[cfg(windows)]
    static PROGRAM_DATA_DIR: Lazy<PathBuf> = Lazy::new(|| {
        PathBuf::from(std::env::var("ProgramData").expect("Could not determine %ProgramData%"))
    });

    /// Configuration directory for windows: `%ProgramData%\distant`.
    #[cfg(windows)]
    static CONFIG_DIR: Lazy<PathBuf> = Lazy::new(|| PROGRAM_DATA_DIR.join("distant"));

    /// Configuration directory for unix: `/etc/distant`.
    #[cfg(unix)]
    static CONFIG_DIR: Lazy<PathBuf> = Lazy::new(|| PathBuf::from("/etc").join("distant"));

    /// Path to configuration settings for distant client/manager/server.
    pub static CONFIG_FILE_PATH: Lazy<PathBuf> = Lazy::new(|| CONFIG_DIR.join("config.toml"));

    /// For Linux & BSD, this uses the runtime path. For Mac, this uses the tmp path
    ///
    /// * `/run/distant.sock` on Linux
    /// * `/var/run/distant.sock` on BSD
    /// * `/tmp/distant.dock` on MacOS
    /// * `@TERMUX_PREFIX@/var/run/distant.sock` on Android (Termux)
    pub static UNIX_SOCKET_PATH: Lazy<PathBuf> = Lazy::new(|| {
        if cfg!(target_os = "macos") {
            std::env::temp_dir().join(SOCKET_FILE_STR)
        } else if cfg!(any(
            target_os = "freebsd",
            target_os = "dragonfly",
            target_os = "openbsd",
            target_os = "netbsd"
        )) {
            PathBuf::from("/var").join("run").join(SOCKET_FILE_STR)
        } else if cfg!(target_os = "android") {
            PathBuf::from("@TERMUX_PREFIX@/var")
                .join("run")
                .join(SOCKET_FILE_STR)
        } else {
            PathBuf::from("/run").join(SOCKET_FILE_STR)
        }
    });

    /// Name of the pipe used by Windows.
    pub static WINDOWS_PIPE_NAME: Lazy<String> = Lazy::new(|| "distant".to_string());
}
