use super::common::{self, LoggingSettings, NetworkSettings};
use serde::{Deserialize, Serialize};

mod api;
mod connect;
mod launch;

pub use api::*;
pub use connect::*;
pub use launch::*;

/// Represents configuration settings for the distant client
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientConfig {
    #[serde(flatten)]
    pub logging: LoggingSettings,

    #[serde(flatten)]
    pub network: NetworkSettings,

    pub api: ClientApiConfig,
    pub connect: ClientConnectConfig,
    pub launch: ClientLaunchConfig,
}
