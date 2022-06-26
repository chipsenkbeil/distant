use clap::Args;
use merge::Merge;
use serde::{Deserialize, Serialize};

/// Represents configuration settings for the distant server
#[derive(Args, Debug, Default, Merge, Serialize, Deserialize)]
pub struct ServerConfig {}
