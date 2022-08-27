use clap::Args;
use serde::{Deserialize, Serialize};

#[derive(Args, Debug, Default, Serialize, Deserialize)]
pub struct ClientActionConfig {
    /// Represents the maximum time (in seconds) to wait for a network request before timing out
    pub timeout: Option<f32>,
}
