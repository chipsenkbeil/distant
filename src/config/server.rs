use super::CommonConfig;
use serde::{Deserialize, Serialize};

mod listen;
pub use listen::*;

/// Represents configuration settings for the distant server
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(flatten)]
    pub common: CommonConfig,

    pub listen: ServerListenConfig,
}
