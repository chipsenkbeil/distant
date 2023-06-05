use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerWatchConfig {
    pub native: bool,
    pub poll_interval: Option<Duration>,
    pub compare_contents: bool,
    pub debounce_timeout: Duration,
    pub debounce_tick_rate: Option<Duration>,
}

impl Default for ServerWatchConfig {
    fn default() -> Self {
        Self {
            native: true,
            poll_interval: None,
            compare_contents: false,
            debounce_timeout: Duration::from_millis(500),
            debounce_tick_rate: None,
        }
    }
}
