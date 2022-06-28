use super::CommonConfig;
use clap::Args;
use merge::Merge;
use serde::{Deserialize, Serialize};

mod listen;
pub use listen::*;

/// Represents configuration settings for the distant server
#[derive(Args, Debug, Default, Merge, Serialize, Deserialize)]
pub struct ServerConfig {
    #[clap(flatten)]
    #[serde(flatten)]
    pub common: CommonConfig,
}
