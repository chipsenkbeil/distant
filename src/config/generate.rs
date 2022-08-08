use super::CommonConfig;
use serde::{Deserialize, Serialize};

/// Represents configuration settings for the distant generate
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GenerateConfig {
    #[serde(flatten)]
    pub common: CommonConfig,
}
