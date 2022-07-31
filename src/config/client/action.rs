use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ClientActionConfig {
    /// Represents the maximum time (in seconds) to wait for a network request before timing out
    pub timeout: Option<f32>,
}
