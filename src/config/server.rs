use super::CommonConfig;
use crate::Merge;
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

impl Merge for ServerConfig {
    fn merge(&mut self, other: Self) {
        self.common.merge(other.common);
        self.listen.merge(other.listen);
    }
}

impl Merge<CommonConfig> for ServerConfig {
    fn merge(&mut self, other: CommonConfig) {
        self.common.merge(other);
    }
}
