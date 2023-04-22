use super::common::{AccessControl, LoggingSettings, NetworkSettings};
use serde::{Deserialize, Serialize};

/// Represents configuration settings for the distant manager
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerConfig {
    #[serde(flatten)]
    pub logging: LoggingSettings,

    #[serde(flatten)]
    pub network: NetworkSettings,

    pub access: Option<AccessControl>,
}
