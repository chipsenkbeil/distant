use std::fmt;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::time::Duration;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

/// Represents a time in seconds.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Seconds(Duration);

impl FromStr for Seconds {
    type Err = ParseSecondsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match f64::from_str(s) {
            Ok(secs) => Ok(Self::try_from(secs)?),
            Err(_) => Err(ParseSecondsError::NotANumber),
        }
    }
}

impl fmt::Display for Seconds {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.as_secs_f32())
    }
}

impl Deref for Seconds {
    type Target = Duration;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Seconds {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl TryFrom<i8> for Seconds {
    type Error = std::num::TryFromIntError;

    fn try_from(secs: i8) -> Result<Self, Self::Error> {
        Ok(Self(Duration::from_secs(u64::try_from(secs)?)))
    }
}

impl TryFrom<i16> for Seconds {
    type Error = std::num::TryFromIntError;

    fn try_from(secs: i16) -> Result<Self, Self::Error> {
        Ok(Self(Duration::from_secs(u64::try_from(secs)?)))
    }
}

impl TryFrom<i32> for Seconds {
    type Error = std::num::TryFromIntError;

    fn try_from(secs: i32) -> Result<Self, Self::Error> {
        Ok(Self(Duration::from_secs(u64::try_from(secs)?)))
    }
}

impl TryFrom<i64> for Seconds {
    type Error = std::num::TryFromIntError;

    fn try_from(secs: i64) -> Result<Self, Self::Error> {
        Ok(Self(Duration::from_secs(u64::try_from(secs)?)))
    }
}

impl From<u8> for Seconds {
    fn from(secs: u8) -> Self {
        Self(Duration::from_secs(u64::from(secs)))
    }
}

impl From<u16> for Seconds {
    fn from(secs: u16) -> Self {
        Self(Duration::from_secs(u64::from(secs)))
    }
}

impl From<u32> for Seconds {
    fn from(secs: u32) -> Self {
        Self(Duration::from_secs(u64::from(secs)))
    }
}

impl From<u64> for Seconds {
    fn from(secs: u64) -> Self {
        Self(Duration::from_secs(secs))
    }
}

impl TryFrom<f32> for Seconds {
    type Error = NegativeSeconds;

    fn try_from(secs: f32) -> Result<Self, Self::Error> {
        if secs.is_sign_negative() {
            Err(NegativeSeconds)
        } else {
            Ok(Self(Duration::from_secs_f32(secs)))
        }
    }
}

impl TryFrom<f64> for Seconds {
    type Error = NegativeSeconds;

    fn try_from(secs: f64) -> Result<Self, Self::Error> {
        if secs.is_sign_negative() {
            Err(NegativeSeconds)
        } else {
            Ok(Self(Duration::from_secs_f64(secs)))
        }
    }
}

impl From<Duration> for Seconds {
    fn from(d: Duration) -> Self {
        Self(d)
    }
}

impl From<Seconds> for Duration {
    fn from(secs: Seconds) -> Self {
        secs.0
    }
}

pub use self::errors::{NegativeSeconds, ParseSecondsError};

mod errors {
    use super::*;

    /// Represents errors that can occur when parsing seconds.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub enum ParseSecondsError {
        NegativeSeconds,
        NotANumber,
    }

    impl std::error::Error for ParseSecondsError {}

    impl fmt::Display for ParseSecondsError {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            match self {
                Self::NegativeSeconds => write!(f, "seconds cannot be negative"),
                Self::NotANumber => write!(f, "seconds must be a number"),
            }
        }
    }

    impl From<NegativeSeconds> for ParseSecondsError {
        fn from(_: NegativeSeconds) -> Self {
            Self::NegativeSeconds
        }
    }

    /// Error type when provided seconds is negative.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct NegativeSeconds;

    impl std::error::Error for NegativeSeconds {}

    impl fmt::Display for NegativeSeconds {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "seconds cannot be negative")
        }
    }
}

mod ser {
    use super::*;

    impl Serialize for Seconds {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_f32(self.as_secs_f32())
        }
    }

    impl<'de> Deserialize<'de> for Seconds {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_f32(SecondsVisitor)
        }
    }

    struct SecondsVisitor;

    impl<'de> de::Visitor<'de> for SecondsVisitor {
        type Value = Seconds;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a valid amount of seconds")
        }

        fn visit_i8<E>(self, value: i8) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Seconds::try_from(value).map_err(de::Error::custom)
        }

        fn visit_i16<E>(self, value: i16) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Seconds::try_from(value).map_err(de::Error::custom)
        }

        fn visit_i32<E>(self, value: i32) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Seconds::try_from(value).map_err(de::Error::custom)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Seconds::try_from(value).map_err(de::Error::custom)
        }

        fn visit_u8<E>(self, value: u8) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Seconds::from(value))
        }

        fn visit_u16<E>(self, value: u16) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Seconds::from(value))
        }

        fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Seconds::from(value))
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(Seconds::from(value))
        }

        fn visit_f32<E>(self, value: f32) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Seconds::try_from(value).map_err(de::Error::custom)
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Seconds::try_from(value).map_err(de::Error::custom)
        }

        fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            s.parse().map_err(de::Error::custom)
        }
    }
}
