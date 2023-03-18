use super::{AccessControl, CommonConfig, NetworkConfig};
use clap::Args;
use serde::{Deserialize, Serialize};

/// Represents configuration settings for the distant manager
#[derive(Args, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerConfig {
    /// Type of access to apply to created unix socket or windows pipe
    #[clap(long, value_enum)]
    pub access: Option<AccessControl>,

    #[clap(flatten)]
    #[serde(flatten)]
    pub common: CommonConfig,

    #[clap(flatten)]
    #[serde(flatten)]
    pub network: NetworkConfig,
}
