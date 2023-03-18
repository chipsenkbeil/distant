use super::CommonConfig;
use serde::{Deserialize, Serialize};

/// Represents configuration settings for the distant generate
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateConfig {
    #[serde(flatten)]
    pub common: CommonConfig,
}
