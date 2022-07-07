use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    /// Do not use a service manager and instead fork the process
    #[cfg(unix)]
    Fork,

    /// Use launchd to manage the service
    #[cfg(target_os = "macos")]
    Launchd,

    /// Use OpenRC to manage the service
    #[cfg(unix)]
    OpenRc,

    /// Use rc to manage the service
    #[cfg(unix)]
    Rc,

    /// Use Windows service controller to manage the service
    #[cfg(windows)]
    Sc,

    /// Use systemd to manage the service
    #[cfg(unix)]
    Systemd,
}

impl Default for ServiceKind {
    #[cfg(target_os = "macos")]
    fn default() -> Self {
        Self::Launchd
    }

    #[cfg(target_os = "windows")]
    fn default() -> Self {
        Self::Sc
    }

    #[cfg(any(
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    fn default() -> Self {
        Self::Rc
    }

    #[cfg(target_os = "linux")]
    fn default() -> Self {
        Self::Systemd
    }
}
