use serde::{Deserialize, Serialize};
use std::{env, path::PathBuf};

/// Represents information about a system
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SystemInfo {
    /// Family of the operating system as described in
    /// https://doc.rust-lang.org/std/env/consts/constant.FAMILY.html
    pub family: String,

    /// Name of the specific operating system as described in
    /// https://doc.rust-lang.org/std/env/consts/constant.OS.html
    pub os: String,

    /// Architecture of the CPI as described in
    /// https://doc.rust-lang.org/std/env/consts/constant.ARCH.html
    pub arch: String,

    /// Current working directory of the running server process
    pub current_dir: PathBuf,

    /// Primary separator for path components for the current platform
    /// as defined in https://doc.rust-lang.org/std/path/constant.MAIN_SEPARATOR.html
    pub main_separator: char,

    /// Name of the user running the server process
    pub username: String,

    /// Default shell tied to user running the server process
    pub shell: String,
}

#[cfg(feature = "schemars")]
impl SystemInfo {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(SystemInfo)
    }
}

impl Default for SystemInfo {
    fn default() -> Self {
        Self {
            family: env::consts::FAMILY.to_string(),
            os: env::consts::OS.to_string(),
            arch: env::consts::ARCH.to_string(),
            current_dir: env::current_dir().unwrap_or_default(),
            main_separator: std::path::MAIN_SEPARATOR,
            username: whoami::username(),
            shell: if cfg!(windows) {
                env::var("ComSpec").unwrap_or_else(|_| String::from("cmd.exe"))
            } else {
                env::var("SHELL").unwrap_or_else(|_| String::from("/bin/sh"))
            },
        }
    }
}
