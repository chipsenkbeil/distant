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

#[cfg(test)]
mod tests {
    //! Tests for `Config` and `WatchConfig` default values, equality, clone,
    //! inequality when fields differ, and custom value round-trip.

    use super::*;

    mod config_default {
        use super::*;

        #[test]
        fn has_default_watch_config() {
            let config = Config::default();
            assert_eq!(config.watch, WatchConfig::default());
        }

        #[test]
        fn equality() {
            let a = Config::default();
            let b = Config::default();
            assert_eq!(a, b);
        }

        #[test]
        fn clone() {
            let a = Config::default();
            let b = a.clone();
            assert_eq!(a, b);
        }
    }

    mod watch_config_default {
        use super::*;

        #[test]
        fn native_is_true() {
            let config = WatchConfig::default();
            assert!(config.native);
        }

        #[test]
        fn poll_interval_is_none() {
            let config = WatchConfig::default();
            assert!(config.poll_interval.is_none());
        }

        #[test]
        fn compare_contents_is_false() {
            let config = WatchConfig::default();
            assert!(!config.compare_contents);
        }

        #[test]
        fn debounce_timeout_is_500ms() {
            let config = WatchConfig::default();
            assert_eq!(config.debounce_timeout, Duration::from_millis(500));
        }

        #[test]
        fn debounce_tick_rate_is_none() {
            let config = WatchConfig::default();
            assert!(config.debounce_tick_rate.is_none());
        }

        #[test]
        fn equality() {
            let a = WatchConfig::default();
            let b = WatchConfig::default();
            assert_eq!(a, b);
        }

        #[test]
        fn clone() {
            let a = WatchConfig::default();
            let b = a.clone();
            assert_eq!(a, b);
        }

        #[test]
        fn inequality_when_fields_differ() {
            let a = WatchConfig::default();
            let b = WatchConfig {
                native: false,
                ..WatchConfig::default()
            };
            assert_ne!(a, b);
        }

        #[test]
        fn custom_values_roundtrip() {
            let config = WatchConfig {
                native: false,
                poll_interval: Some(Duration::from_secs(5)),
                compare_contents: true,
                debounce_timeout: Duration::from_millis(100),
                debounce_tick_rate: Some(Duration::from_millis(50)),
            };
            let cloned = config.clone();
            assert_eq!(config, cloned);
            assert!(!cloned.native);
            assert_eq!(cloned.poll_interval, Some(Duration::from_secs(5)));
            assert!(cloned.compare_contents);
            assert_eq!(cloned.debounce_timeout, Duration::from_millis(100));
            assert_eq!(cloned.debounce_tick_rate, Some(Duration::from_millis(50)));
        }
    }
}
