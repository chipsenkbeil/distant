use std::num::ParseFloatError;
use std::str::FromStr;
use std::time::Duration;

use derive_more::{Display, Error};
use serde::{Deserialize, Serialize};

const DEFAULT_CONNECTION_SLEEP: Duration = Duration::from_millis(1);
const DEFAULT_HEARTBEAT_DURATION: Duration = Duration::from_secs(5);

/// Represents a general-purpose set of properties tied with a server instance
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Time to wait inbetween connection read/write when nothing was read or written on last pass
    pub connection_sleep: Duration,

    /// Minimum time to wait inbetween sending heartbeat messages
    pub connection_heartbeat: Duration,

    /// Rules for how a server will shutdown automatically
    pub shutdown: Shutdown,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            connection_sleep: DEFAULT_CONNECTION_SLEEP,
            connection_heartbeat: DEFAULT_HEARTBEAT_DURATION,
            shutdown: Default::default(),
        }
    }
}

/// Rules for how a server will shut itself down automatically
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq)]
pub enum Shutdown {
    /// Server should shutdown immediately after duration exceeded
    #[display(fmt = "after={}", "_0.as_secs_f32()")]
    After(Duration),

    /// Server should shutdown after no connections for over duration time
    #[display(fmt = "lonely={}", "_0.as_secs_f32()")]
    Lonely(Duration),

    /// No shutdown logic will be applied to the server
    #[display(fmt = "never")]
    Never,
}

impl Shutdown {
    /// Return duration associated with shutdown if it has one
    pub fn duration(&self) -> Option<Duration> {
        match self {
            Self::Never => None,
            Self::After(x) | Self::Lonely(x) => Some(*x),
        }
    }
}

impl Default for Shutdown {
    /// By default, shutdown is never
    fn default() -> Self {
        Self::Never
    }
}

/// Parsing errors that can occur for [`Shutdown`]
#[derive(Clone, Debug, Display, Error, PartialEq, Eq)]
pub enum ShutdownParseError {
    #[display(fmt = "Bad value for after: {_0}")]
    BadValueForAfter(ParseFloatError),

    #[display(fmt = "Bad value for lonely: {_0}")]
    BadValueForLonely(ParseFloatError),

    #[display(fmt = "Missing key")]
    MissingKey,

    #[display(fmt = "Unknown key")]
    UnknownKey,
}

impl FromStr for Shutdown {
    type Err = ShutdownParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("never") {
            Ok(Self::Never)
        } else {
            let (left, right) = s.split_once('=').ok_or(ShutdownParseError::MissingKey)?;
            let left = left.trim();
            let right = right.trim();
            if left.eq_ignore_ascii_case("after") {
                Ok(Self::After(Duration::from_secs_f32(
                    right
                        .parse()
                        .map_err(ShutdownParseError::BadValueForAfter)?,
                )))
            } else if left.eq_ignore_ascii_case("lonely") {
                Ok(Self::Lonely(Duration::from_secs_f32(
                    right
                        .parse()
                        .map_err(ShutdownParseError::BadValueForLonely)?,
                )))
            } else {
                Err(ShutdownParseError::UnknownKey)
            }
        }
    }
}

impl Serialize for Shutdown {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        String::serialize(&self.to_string(), serializer)
    }
}

impl<'de> Deserialize<'de> for Shutdown {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    //! Tests for ServerConfig defaults and serde, Shutdown Display/FromStr/duration/serde
    //! (including case insensitivity, fractional seconds, whitespace tolerance),
    //! and ShutdownParseError Display.

    use test_log::test;

    use super::*;

    // ---- ServerConfig Default ----

    #[test]
    fn server_config_default_has_expected_connection_sleep() {
        let config = ServerConfig::default();
        assert_eq!(config.connection_sleep, Duration::from_millis(1));
    }

    #[test]
    fn server_config_default_has_expected_connection_heartbeat() {
        let config = ServerConfig::default();
        assert_eq!(config.connection_heartbeat, Duration::from_secs(5));
    }

    #[test]
    fn server_config_default_has_never_shutdown() {
        let config = ServerConfig::default();
        assert_eq!(config.shutdown, Shutdown::Never);
    }

    // ---- ServerConfig serde round-trip ----

    #[test]
    fn server_config_should_serialize_and_deserialize_with_defaults() {
        let config = ServerConfig::default();
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: ServerConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn server_config_should_serialize_and_deserialize_with_after_shutdown() {
        let config = ServerConfig {
            connection_sleep: Duration::from_millis(50),
            connection_heartbeat: Duration::from_secs(10),
            shutdown: Shutdown::After(Duration::from_secs(30)),
        };
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: ServerConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn server_config_should_serialize_and_deserialize_with_lonely_shutdown() {
        let config = ServerConfig {
            connection_sleep: Duration::from_millis(10),
            connection_heartbeat: Duration::from_secs(3),
            shutdown: Shutdown::Lonely(Duration::from_secs(60)),
        };
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: ServerConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }

    // ---- Shutdown Display ----

    #[test]
    fn shutdown_display_after_shows_seconds() {
        let s = Shutdown::After(Duration::from_secs(5));
        assert_eq!(s.to_string(), "after=5");
    }

    #[test]
    fn shutdown_display_after_shows_fractional_seconds() {
        let s = Shutdown::After(Duration::from_millis(1500));
        assert_eq!(s.to_string(), "after=1.5");
    }

    #[test]
    fn shutdown_display_lonely_shows_seconds() {
        let s = Shutdown::Lonely(Duration::from_secs(10));
        assert_eq!(s.to_string(), "lonely=10");
    }

    #[test]
    fn shutdown_display_lonely_shows_fractional_seconds() {
        let s = Shutdown::Lonely(Duration::from_millis(2500));
        assert_eq!(s.to_string(), "lonely=2.5");
    }

    #[test]
    fn shutdown_display_never() {
        let s = Shutdown::Never;
        assert_eq!(s.to_string(), "never");
    }

    // ---- Shutdown FromStr ----

    #[test]
    fn shutdown_from_str_parses_never() {
        let s: Shutdown = "never".parse().unwrap();
        assert_eq!(s, Shutdown::Never);
    }

    #[test]
    fn shutdown_from_str_parses_never_case_insensitive() {
        let s: Shutdown = "NEVER".parse().unwrap();
        assert_eq!(s, Shutdown::Never);

        let s2: Shutdown = "Never".parse().unwrap();
        assert_eq!(s2, Shutdown::Never);
    }

    #[test]
    fn shutdown_from_str_parses_after() {
        let s: Shutdown = "after=5".parse().unwrap();
        assert_eq!(s, Shutdown::After(Duration::from_secs(5)));
    }

    #[test]
    fn shutdown_from_str_parses_after_case_insensitive() {
        let s: Shutdown = "AFTER=5".parse().unwrap();
        assert_eq!(s, Shutdown::After(Duration::from_secs(5)));
    }

    #[test]
    fn shutdown_from_str_parses_after_with_fractional_seconds() {
        let s: Shutdown = "after=1.5".parse().unwrap();
        assert_eq!(s, Shutdown::After(Duration::from_secs_f32(1.5)));
    }

    #[test]
    fn shutdown_from_str_parses_lonely() {
        let s: Shutdown = "lonely=10".parse().unwrap();
        assert_eq!(s, Shutdown::Lonely(Duration::from_secs(10)));
    }

    #[test]
    fn shutdown_from_str_parses_lonely_case_insensitive() {
        let s: Shutdown = "LONELY=10".parse().unwrap();
        assert_eq!(s, Shutdown::Lonely(Duration::from_secs(10)));
    }

    #[test]
    fn shutdown_from_str_parses_lonely_with_fractional_seconds() {
        let s: Shutdown = "lonely=2.5".parse().unwrap();
        assert_eq!(s, Shutdown::Lonely(Duration::from_secs_f32(2.5)));
    }

    #[test]
    fn shutdown_from_str_parses_with_whitespace_around_equals() {
        let s: Shutdown = "after = 5".parse().unwrap();
        assert_eq!(s, Shutdown::After(Duration::from_secs(5)));
    }

    #[test]
    fn shutdown_from_str_fails_with_missing_key() {
        let err = "noequals".parse::<Shutdown>().unwrap_err();
        assert_eq!(err, ShutdownParseError::MissingKey);
    }

    #[test]
    fn shutdown_from_str_fails_with_unknown_key() {
        let err = "unknown=5".parse::<Shutdown>().unwrap_err();
        assert_eq!(err, ShutdownParseError::UnknownKey);
    }

    #[test]
    fn shutdown_from_str_fails_with_bad_value_for_after() {
        let err = "after=abc".parse::<Shutdown>().unwrap_err();
        assert!(matches!(err, ShutdownParseError::BadValueForAfter(_)));
    }

    #[test]
    fn shutdown_from_str_fails_with_bad_value_for_lonely() {
        let err = "lonely=abc".parse::<Shutdown>().unwrap_err();
        assert!(matches!(err, ShutdownParseError::BadValueForLonely(_)));
    }

    // ---- Shutdown duration() ----

    #[test]
    fn shutdown_duration_returns_some_for_after() {
        let s = Shutdown::After(Duration::from_secs(5));
        assert_eq!(s.duration(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn shutdown_duration_returns_some_for_lonely() {
        let s = Shutdown::Lonely(Duration::from_secs(10));
        assert_eq!(s.duration(), Some(Duration::from_secs(10)));
    }

    #[test]
    fn shutdown_duration_returns_none_for_never() {
        let s = Shutdown::Never;
        assert_eq!(s.duration(), None);
    }

    // ---- Shutdown Default ----

    #[test]
    fn shutdown_default_is_never() {
        assert_eq!(Shutdown::default(), Shutdown::Never);
    }

    // ---- Shutdown serde round-trip ----

    #[test]
    fn shutdown_serde_round_trip_never() {
        let s = Shutdown::Never;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"never\"");
        let deserialized: Shutdown = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, s);
    }

    #[test]
    fn shutdown_serde_round_trip_after() {
        let s = Shutdown::After(Duration::from_secs(5));
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"after=5\"");
        let deserialized: Shutdown = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, s);
    }

    #[test]
    fn shutdown_serde_round_trip_lonely() {
        let s = Shutdown::Lonely(Duration::from_secs(10));
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"lonely=10\"");
        let deserialized: Shutdown = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, s);
    }

    // ---- ShutdownParseError Display ----

    #[test]
    fn shutdown_parse_error_display_bad_value_for_after() {
        let inner = "abc".parse::<f32>().unwrap_err();
        let err = ShutdownParseError::BadValueForAfter(inner.clone());
        assert_eq!(err.to_string(), format!("Bad value for after: {inner}"));
    }

    #[test]
    fn shutdown_parse_error_display_bad_value_for_lonely() {
        let inner = "xyz".parse::<f32>().unwrap_err();
        let err = ShutdownParseError::BadValueForLonely(inner.clone());
        assert_eq!(err.to_string(), format!("Bad value for lonely: {inner}"));
    }

    #[test]
    fn shutdown_parse_error_display_missing_key() {
        let err = ShutdownParseError::MissingKey;
        assert_eq!(err.to_string(), "Missing key");
    }

    #[test]
    fn shutdown_parse_error_display_unknown_key() {
        let err = ShutdownParseError::UnknownKey;
        assert_eq!(err.to_string(), "Unknown key");
    }
}
