use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub watch: WatchConfig,
}

/// Configuration specifically for watching files and directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchConfig {
    pub native: bool,
    pub poll_interval: Option<Duration>,
    pub compare_contents: bool,
    pub debounce_timeout: Duration,
    pub debounce_tick_rate: Option<Duration>,
}

impl Default for WatchConfig {
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
