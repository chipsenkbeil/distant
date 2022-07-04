use super::{CommonConfig, NetworkConfig};
use crate::Merge;
use serde::{Deserialize, Serialize};

mod launch;
pub use launch::*;

/// Represents configuration settings for the distant client
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(flatten)]
    pub common: CommonConfig,

    pub launch: ClientLaunchConfig,

    #[serde(flatten)]
    pub network: NetworkConfig,
}

impl Merge for ClientConfig {
    fn merge(&mut self, other: Self) {
        self.common.merge(other.common);
        self.launch.merge(other.launch);
        self.network.merge(other.network);
    }
}

impl Merge<CommonConfig> for ClientConfig {
    fn merge(&mut self, other: CommonConfig) {
        self.common.merge(other);
    }
}

impl Merge<ClientLaunchConfig> for ClientConfig {
    fn merge(&mut self, other: ClientLaunchConfig) {
        self.launch.merge(other);
    }
}

impl Merge<NetworkConfig> for ClientConfig {
    fn merge(&mut self, other: NetworkConfig) {
        self.network.merge(other);
    }
}
