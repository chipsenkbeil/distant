use clap::Args;
use serde::{Deserialize, Serialize};

#[derive(Args, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ClientReplConfig {
    /// Represents the maximum time (in seconds) to wait for a network request before timing out
    pub timeout: Option<f32>,
}
