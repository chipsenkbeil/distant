use super::{CommonConfig, NetworkConfig};
use serde::{Deserialize, Serialize};

mod action;
mod launch;
mod repl;

pub use action::*;
pub use launch::*;
pub use repl::*;

/// Represents configuration settings for the distant client
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(flatten)]
    pub common: CommonConfig,

    pub action: ClientActionConfig,
    pub launch: ClientLaunchConfig,
    pub repl: ClientReplConfig,

    #[serde(flatten)]
    pub network: NetworkConfig,
}
