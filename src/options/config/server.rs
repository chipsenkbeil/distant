use super::common::LoggingSettings;
use serde::{Deserialize, Serialize};

mod listen;
pub use listen::*;

/// Represents configuration settings for the distant server
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(flatten)]
    pub logging: LoggingSettings,

    pub listen: ServerListenConfig,
}
