use crate::options::common::Seconds;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerWatchConfig {
    pub native: bool,
    pub poll_interval: Option<Seconds>,
    pub compare_contents: bool,
    pub debounce_timeout: Option<Seconds>,
    pub debounce_tick_rate: Option<Seconds>,
}

impl Default for ServerWatchConfig {
    fn default() -> Self {
        Self {
            native: true,
            poll_interval: None,
            compare_contents: false,
            debounce_timeout: None,
            debounce_tick_rate: None,
        }
    }
}
