use super::common::{self, LoggingSettings, NetworkSettings};
use serde::{Deserialize, Serialize};

mod action;
mod connect;
mod launch;
mod repl;

pub use action::*;
pub use connect::*;
pub use launch::*;
pub use repl::*;

/// Represents configuration settings for the distant client
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(flatten)]
    pub logging: LoggingSettings,

    #[serde(flatten)]
    pub network: NetworkSettings,

    pub action: ClientActionConfig,
    pub connect: ClientConnectConfig,
    pub launch: ClientLaunchConfig,
    pub repl: ClientReplConfig,
}
