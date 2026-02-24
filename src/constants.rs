#![allow(dead_code)]

use std::path::PathBuf;

use directories::ProjectDirs;
use once_cell::sync::Lazy;

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

#[cfg(test)]
mod tests {
    //! Tests for module-level constants (`MAX_PIPE_CHUNK_SIZE`, `SOCKET_FILE_STR`)
    //! and the `Lazy`-initialized path/name constants in `user` and `global` sub-modules.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // MAX_PIPE_CHUNK_SIZE
    // -------------------------------------------------------
    #[test]
    fn max_pipe_chunk_size_is_16k() {
        assert_eq!(MAX_PIPE_CHUNK_SIZE, 16384);
        assert_eq!(MAX_PIPE_CHUNK_SIZE, 16 * 1024);
    }

    // -------------------------------------------------------
    // SOCKET_FILE_STR
    // -------------------------------------------------------
    #[test]
    fn socket_file_str_is_distant_sock() {
        assert_eq!(SOCKET_FILE_STR, "distant.sock");
    }

    // -------------------------------------------------------
    // user paths are non-empty
    // -------------------------------------------------------
    #[test]
    fn user_config_file_path_ends_with_config_toml() {
        let path = user::CONFIG_FILE_PATH.as_path();
        assert!(
            path.ends_with("config.toml"),
            "Expected path to end with config.toml, got: {path:?}"
        );
    }

    #[test]
    fn user_cache_file_path_ends_with_cache_toml() {
        let path = user::CACHE_FILE_PATH.as_path();
        assert!(
            path.ends_with("cache.toml"),
            "Expected path to end with cache.toml, got: {path:?}"
        );
    }

    #[test]
    fn user_cache_file_path_str_matches_path() {
        let path_str = user::CACHE_FILE_PATH_STR.as_str();
        let path = user::CACHE_FILE_PATH.to_string_lossy();
        assert_eq!(path_str, &*path);
    }

    #[test]
    fn user_client_log_file_path_ends_with_client_log() {
        let path = user::CLIENT_LOG_FILE_PATH.as_path();
        assert!(
            path.ends_with("client.log"),
            "Expected path to end with client.log, got: {path:?}"
        );
    }

    #[test]
    fn user_manager_log_file_path_ends_with_manager_log() {
        let path = user::MANAGER_LOG_FILE_PATH.as_path();
        assert!(
            path.ends_with("manager.log"),
            "Expected path to end with manager.log, got: {path:?}"
        );
    }

    #[test]
    fn user_server_log_file_path_ends_with_server_log() {
        let path = user::SERVER_LOG_FILE_PATH.as_path();
        assert!(
            path.ends_with("server.log"),
            "Expected path to end with server.log, got: {path:?}"
        );
    }

    #[test]
    fn user_generate_log_file_path_ends_with_generate_log() {
        let path = user::GENERATE_LOG_FILE_PATH.as_path();
        assert!(
            path.ends_with("generate.log"),
            "Expected path to end with generate.log, got: {path:?}"
        );
    }

    #[test]
    fn user_unix_socket_path_contains_distant_sock() {
        let path = user::UNIX_SOCKET_PATH.as_path();
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            file_name.ends_with("distant.sock"),
            "Expected file name to end with distant.sock, got: {file_name}"
        );
    }

    #[test]
    fn user_windows_pipe_name_contains_distant() {
        let name = user::WINDOWS_PIPE_NAME.as_str();
        assert!(
            name.ends_with(".distant"),
            "Expected pipe name to end with .distant, got: {name}"
        );
    }

    // -------------------------------------------------------
    // global paths
    // -------------------------------------------------------
    #[test]
    fn global_config_file_path_ends_with_config_toml() {
        let path = global::CONFIG_FILE_PATH.as_path();
        assert!(
            path.ends_with("config.toml"),
            "Expected path to end with config.toml, got: {path:?}"
        );
    }

    #[test]
    fn global_unix_socket_path_ends_with_distant_sock() {
        let path = global::UNIX_SOCKET_PATH.as_path();
        assert!(
            path.ends_with("distant.sock"),
            "Expected path to end with distant.sock, got: {path:?}"
        );
    }

    #[test]
    fn global_windows_pipe_name_is_distant() {
        assert_eq!(global::WINDOWS_PIPE_NAME.as_str(), "distant");
    }
}
