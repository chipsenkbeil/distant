use super::common::LoggingSettings;
use serde::{Deserialize, Serialize};

/// Represents configuration settings for the distant generate
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateConfig {
    #[serde(flatten)]
    pub logging: LoggingSettings,
}
