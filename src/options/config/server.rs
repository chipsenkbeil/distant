use serde::{Deserialize, Serialize};

use super::common::LoggingSettings;

mod listen;
mod watch;

pub use listen::*;
pub use watch::*;

/// Represents configuration settings for the distant server
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(flatten)]
    pub logging: LoggingSettings,

    pub listen: ServerListenConfig,
    pub watch: ServerWatchConfig,
}
