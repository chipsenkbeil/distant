use std::path::PathBuf;

use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};

/// Contains settings are associated with logging.
#[derive(Args, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggingSettings {
    /// Log level to use throughout the application
    #[clap(long, global = true, value_enum)]
    pub log_level: Option<LogLevel>,

    /// Path to file to use for logging
    #[clap(long, global = true)]
    pub log_file: Option<PathBuf>,
}

impl LoggingSettings {
    pub fn log_level_or_default(&self) -> LogLevel {
        self.log_level.as_ref().copied().unwrap_or_default()
    }
}

/// Represents the level associated with logging.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[clap(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum LogLevel {
    Off,
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn to_log_level_filter(self) -> log::LevelFilter {
        match self {
            Self::Off => log::LevelFilter::Off,
            Self::Error => log::LevelFilter::Error,
            Self::Warn => log::LevelFilter::Warn,
            Self::Info => log::LevelFilter::Info,
            Self::Debug => log::LevelFilter::Debug,
            Self::Trace => log::LevelFilter::Trace,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `LogLevel` enum conversions, defaults, serde, and
    //! `LoggingSettings` accessors and serialization round-trips.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // LogLevel::default
    // -------------------------------------------------------
    #[test]
    fn log_level_default_is_info() {
        assert_eq!(LogLevel::default(), LogLevel::Info);
    }

    // -------------------------------------------------------
    // LogLevel::to_log_level_filter
    // -------------------------------------------------------
    #[test]
    fn to_log_level_filter_off() {
        assert_eq!(LogLevel::Off.to_log_level_filter(), log::LevelFilter::Off);
    }

    #[test]
    fn to_log_level_filter_error() {
        assert_eq!(
            LogLevel::Error.to_log_level_filter(),
            log::LevelFilter::Error
        );
    }

    #[test]
    fn to_log_level_filter_warn() {
        assert_eq!(LogLevel::Warn.to_log_level_filter(), log::LevelFilter::Warn);
    }

    #[test]
    fn to_log_level_filter_info() {
        assert_eq!(LogLevel::Info.to_log_level_filter(), log::LevelFilter::Info);
    }

    #[test]
    fn to_log_level_filter_debug() {
        assert_eq!(
            LogLevel::Debug.to_log_level_filter(),
            log::LevelFilter::Debug
        );
    }

    #[test]
    fn to_log_level_filter_trace() {
        assert_eq!(
            LogLevel::Trace.to_log_level_filter(),
            log::LevelFilter::Trace
        );
    }

    // -------------------------------------------------------
    // LoggingSettings::default
    // -------------------------------------------------------
    #[test]
    fn logging_settings_default_has_no_values() {
        let settings = LoggingSettings::default();
        assert!(settings.log_level.is_none());
        assert!(settings.log_file.is_none());
    }

    // -------------------------------------------------------
    // LoggingSettings::log_level_or_default
    // -------------------------------------------------------
    #[test]
    fn log_level_or_default_returns_set_value() {
        let settings = LoggingSettings {
            log_level: Some(LogLevel::Trace),
            log_file: None,
        };
        assert_eq!(settings.log_level_or_default(), LogLevel::Trace);
    }

    #[test]
    fn log_level_or_default_returns_info_when_none() {
        let settings = LoggingSettings {
            log_level: None,
            log_file: None,
        };
        assert_eq!(settings.log_level_or_default(), LogLevel::Info);
    }

    #[test]
    fn log_level_or_default_works_for_all_variants() {
        for level in [
            LogLevel::Off,
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            let settings = LoggingSettings {
                log_level: Some(level),
                log_file: None,
            };
            assert_eq!(settings.log_level_or_default(), level);
        }
    }

    // -------------------------------------------------------
    // LogLevel serde round-trip
    // -------------------------------------------------------
    #[test]
    fn log_level_serde_round_trip() {
        for level in [
            LogLevel::Off,
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let restored: LogLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(level, restored);
        }
    }

    #[test]
    fn log_level_serializes_to_snake_case() {
        assert_eq!(serde_json::to_string(&LogLevel::Off).unwrap(), r#""off""#);
        assert_eq!(
            serde_json::to_string(&LogLevel::Error).unwrap(),
            r#""error""#
        );
        assert_eq!(serde_json::to_string(&LogLevel::Warn).unwrap(), r#""warn""#);
        assert_eq!(serde_json::to_string(&LogLevel::Info).unwrap(), r#""info""#);
        assert_eq!(
            serde_json::to_string(&LogLevel::Debug).unwrap(),
            r#""debug""#
        );
        assert_eq!(
            serde_json::to_string(&LogLevel::Trace).unwrap(),
            r#""trace""#
        );
    }

    // -------------------------------------------------------
    // LoggingSettings serde round-trip
    // -------------------------------------------------------
    #[test]
    fn logging_settings_serde_round_trip() {
        let settings = LoggingSettings {
            log_level: Some(LogLevel::Debug),
            log_file: Some(PathBuf::from("/var/log/test.log")),
        };
        let json = serde_json::to_string(&settings).unwrap();
        let restored: LoggingSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, restored);
    }

    #[test]
    fn logging_settings_serde_round_trip_empty() {
        let settings = LoggingSettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let restored: LoggingSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, restored);
    }

    // -------------------------------------------------------
    // LogLevel is Copy
    // -------------------------------------------------------
    #[test]
    fn log_level_is_copy() {
        let a = LogLevel::Debug;
        let b = a; // copy
        assert_eq!(a, b);
    }
}
