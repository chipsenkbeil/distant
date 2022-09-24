use crate::auth::MethodType;
use derive_more::{Display, Error};
use serde::{Deserialize, Serialize};
use std::{num::ParseFloatError, str::FromStr, time::Duration};

/// Represents a general-purpose set of properties tied with a server instance
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Supported authentication methods
    pub authentication_methods: Vec<MethodType>,

    /// If true, authentication type "none" is allowed, meaning that no authentication is required
    pub allow_none_authentication: bool,

    /// Rules for how a server will shutdown automatically
    pub shutdown: Shutdown,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            authentication_methods: vec![MethodType::None],
            allow_none_authentication: true,
            shutdown: Shutdown::default(),
        }
    }
}

/// Rules for how a server will shut itself down automatically
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
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
    #[display(fmt = "Bad value for after: {}", _0)]
    BadValueForAfter(ParseFloatError),

    #[display(fmt = "Bad value for lonely: {}", _0)]
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
