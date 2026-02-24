use serde::{Deserialize, Serialize};

use crate::options::common::Seconds;

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

#[cfg(test)]
mod tests {
    //! Tests for `ServerWatchConfig`: defaults (native, compare_contents, optional
    //! fields), serde round-trips, equality, and clone.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // Default
    // -------------------------------------------------------
    #[test]
    fn default_native_is_true() {
        let config = ServerWatchConfig::default();
        assert!(config.native);
    }

    #[test]
    fn default_compare_contents_is_false() {
        let config = ServerWatchConfig::default();
        assert!(!config.compare_contents);
    }

    #[test]
    fn default_optional_fields_are_none() {
        let config = ServerWatchConfig::default();
        assert!(config.poll_interval.is_none());
        assert!(config.debounce_timeout.is_none());
        assert!(config.debounce_tick_rate.is_none());
    }

    // -------------------------------------------------------
    // Serde round-trip
    // -------------------------------------------------------
    #[test]
    fn serde_round_trip_default() {
        let config = ServerWatchConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let restored: ServerWatchConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, restored);
    }

    #[test]
    fn serde_round_trip_with_all_fields() {
        let config = ServerWatchConfig {
            native: false,
            poll_interval: Some(Seconds::from(5u32)),
            compare_contents: true,
            debounce_timeout: Some(Seconds::from(2u32)),
            debounce_tick_rate: Some(Seconds::from(1u32)),
        };
        let json = serde_json::to_string(&config).unwrap();
        let restored: ServerWatchConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, restored);
    }

    // -------------------------------------------------------
    // PartialEq
    // -------------------------------------------------------
    #[test]
    fn equality() {
        let a = ServerWatchConfig::default();
        let b = ServerWatchConfig::default();
        assert_eq!(a, b);

        let c = ServerWatchConfig {
            native: false,
            ..Default::default()
        };
        assert_ne!(a, c);
    }

    // -------------------------------------------------------
    // Clone
    // -------------------------------------------------------
    #[test]
    fn clone_is_equal() {
        let config = ServerWatchConfig {
            native: false,
            poll_interval: Some(Seconds::from(10u32)),
            compare_contents: true,
            debounce_timeout: Some(Seconds::from(3u32)),
            debounce_tick_rate: Some(Seconds::from(1u32)),
        };
        let cloned = config.clone();
        assert_eq!(config, cloned);
    }
}
