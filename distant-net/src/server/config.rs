use derive_more::{Display, Error};
use serde::{Deserialize, Serialize};
use std::{num::ParseFloatError, str::FromStr, time::Duration};

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
