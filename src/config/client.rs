use super::{CommonConfig, NetworkConfig};
use clap::Args;
use merge::Merge;
use serde::{Deserialize, Serialize};

/// Represents configuration settings for the distant client
#[derive(Args, Debug, Default, Merge, Serialize, Deserialize)]
pub struct ClientConfig {
    #[clap(flatten)]
    #[serde(flatten)]
    pub common: CommonConfig,

    #[clap(flatten)]
    #[serde(flatten)]
    pub network: NetworkConfig,
}
