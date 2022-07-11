use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
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
