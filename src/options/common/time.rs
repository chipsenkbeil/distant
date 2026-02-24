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

#[cfg(test)]
mod tests {
    //! Tests for the `Seconds` newtype: `FromStr`, `Display`, `Deref`/`DerefMut`,
    //! `TryFrom` (signed ints, floats), `From` (unsigned ints, `Duration`), error
    //! types, serde (JSON + TOML including custom visitor paths), `Hash`/`Eq`,
    //! and `Copy`/`Clone`.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // FromStr
    // -------------------------------------------------------
    #[test]
    fn from_str_integer_seconds() {
        let s: Seconds = "5".parse().unwrap();
        assert_eq!(*s, Duration::from_secs(5));
    }

    #[test]
    fn from_str_fractional_seconds() {
        let s: Seconds = "1.5".parse().unwrap();
        assert_eq!(*s, Duration::from_secs_f64(1.5));
    }

    #[test]
    fn from_str_zero() {
        let s: Seconds = "0".parse().unwrap();
        assert_eq!(*s, Duration::ZERO);
    }

    #[test]
    fn from_str_negative_fails() {
        let result = "-1".parse::<Seconds>();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ParseSecondsError::NegativeSeconds);
    }

    #[test]
    fn from_str_not_a_number_fails() {
        let result = "abc".parse::<Seconds>();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ParseSecondsError::NotANumber);
    }

    #[test]
    fn from_str_empty_fails() {
        let result = "".parse::<Seconds>();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ParseSecondsError::NotANumber);
    }

    // -------------------------------------------------------
    // Display
    // -------------------------------------------------------
    #[test]
    fn display_shows_float_seconds() {
        let s = Seconds::from(5u32);
        // as_secs_f32 of 5 seconds should print "5"
        let display = s.to_string();
        assert!(display.starts_with("5"), "got: {display}");
    }

    #[test]
    fn display_fractional() {
        let s = Seconds::try_from(1.5f64).unwrap();
        let display = s.to_string();
        assert!(display.starts_with("1.5"), "got: {display}");
    }

    // -------------------------------------------------------
    // Deref / DerefMut
    // -------------------------------------------------------
    #[test]
    fn deref_gives_duration() {
        let s = Seconds::from(10u64);
        let d: &Duration = &s;
        assert_eq!(d.as_secs(), 10);
    }

    #[test]
    fn deref_mut_allows_modification() {
        let mut s = Seconds::from(10u64);
        *s = Duration::from_secs(20);
        assert_eq!(s.as_secs(), 20);
    }

    // -------------------------------------------------------
    // TryFrom signed integers
    // -------------------------------------------------------
    #[test]
    fn try_from_i8_positive() {
        let s = Seconds::try_from(5i8).unwrap();
        assert_eq!(*s, Duration::from_secs(5));
    }

    #[test]
    fn try_from_i8_negative_fails() {
        assert!(Seconds::try_from(-1i8).is_err());
    }

    #[test]
    fn try_from_i16_positive() {
        let s = Seconds::try_from(100i16).unwrap();
        assert_eq!(*s, Duration::from_secs(100));
    }

    #[test]
    fn try_from_i16_negative_fails() {
        assert!(Seconds::try_from(-1i16).is_err());
    }

    #[test]
    fn try_from_i32_positive() {
        let s = Seconds::try_from(3600i32).unwrap();
        assert_eq!(*s, Duration::from_secs(3600));
    }

    #[test]
    fn try_from_i32_negative_fails() {
        assert!(Seconds::try_from(-1i32).is_err());
    }

    #[test]
    fn try_from_i64_positive() {
        let s = Seconds::try_from(86400i64).unwrap();
        assert_eq!(*s, Duration::from_secs(86400));
    }

    #[test]
    fn try_from_i64_negative_fails() {
        assert!(Seconds::try_from(-1i64).is_err());
    }

    // -------------------------------------------------------
    // From unsigned integers
    // -------------------------------------------------------
    #[test]
    fn from_u8() {
        let s = Seconds::from(42u8);
        assert_eq!(*s, Duration::from_secs(42));
    }

    #[test]
    fn from_u16() {
        let s = Seconds::from(1000u16);
        assert_eq!(*s, Duration::from_secs(1000));
    }

    #[test]
    fn from_u32() {
        let s = Seconds::from(60000u32);
        assert_eq!(*s, Duration::from_secs(60000));
    }

    #[test]
    fn from_u64() {
        let s = Seconds::from(1_000_000u64);
        assert_eq!(*s, Duration::from_secs(1_000_000));
    }

    // -------------------------------------------------------
    // TryFrom floats
    // -------------------------------------------------------
    #[test]
    fn try_from_f32_positive() {
        let s = Seconds::try_from(2.5f32).unwrap();
        assert_eq!(*s, Duration::from_secs_f32(2.5));
    }

    #[test]
    fn try_from_f32_zero() {
        let s = Seconds::try_from(0.0f32).unwrap();
        assert_eq!(*s, Duration::ZERO);
    }

    #[test]
    fn try_from_f32_negative_fails() {
        let result = Seconds::try_from(-0.1f32);
        assert!(result.is_err());
    }

    #[test]
    fn try_from_f64_positive() {
        let s = Seconds::try_from(2.75f64).unwrap();
        assert_eq!(*s, Duration::from_secs_f64(2.75));
    }

    #[test]
    fn try_from_f64_zero() {
        let s = Seconds::try_from(0.0f64).unwrap();
        assert_eq!(*s, Duration::ZERO);
    }

    #[test]
    fn try_from_f64_negative_fails() {
        let result = Seconds::try_from(-0.1f64);
        assert!(result.is_err());
    }

    // -------------------------------------------------------
    // From / Into Duration
    // -------------------------------------------------------
    #[test]
    fn from_duration() {
        let d = Duration::from_secs(7);
        let s = Seconds::from(d);
        assert_eq!(*s, d);
    }

    #[test]
    fn into_duration() {
        let s = Seconds::from(7u64);
        let d: Duration = s.into();
        assert_eq!(d, Duration::from_secs(7));
    }

    // -------------------------------------------------------
    // Error types
    // -------------------------------------------------------
    #[test]
    fn parse_seconds_error_display() {
        assert_eq!(
            ParseSecondsError::NegativeSeconds.to_string(),
            "seconds cannot be negative"
        );
        assert_eq!(
            ParseSecondsError::NotANumber.to_string(),
            "seconds must be a number"
        );
    }

    #[test]
    fn negative_seconds_error_display() {
        assert_eq!(NegativeSeconds.to_string(), "seconds cannot be negative");
    }

    #[test]
    fn negative_seconds_converts_to_parse_error() {
        let err: ParseSecondsError = NegativeSeconds.into();
        assert_eq!(err, ParseSecondsError::NegativeSeconds);
    }

    // -------------------------------------------------------
    // Serialization (serde)
    // -------------------------------------------------------
    #[test]
    fn serialize_seconds_to_json() {
        let s = Seconds::from(5u32);
        let json = serde_json::to_string(&s).unwrap();
        // Should serialize as a float
        let val: f64 = serde_json::from_str(&json).unwrap();
        assert!((val - 5.0).abs() < 0.01);
    }

    #[test]
    fn deserialize_seconds_from_integer_json() {
        let s: Seconds = serde_json::from_str("10").unwrap();
        assert_eq!(*s, Duration::from_secs(10));
    }

    #[test]
    fn deserialize_seconds_from_float_json() {
        let s: Seconds = serde_json::from_str("2.5").unwrap();
        assert_eq!(*s, Duration::from_secs_f64(2.5));
    }

    #[test]
    fn deserialize_seconds_from_zero_json() {
        let s: Seconds = serde_json::from_str("0").unwrap();
        assert_eq!(*s, Duration::ZERO);
    }

    #[test]
    fn deserialize_negative_seconds_fails() {
        let result: Result<Seconds, _> = serde_json::from_str("-5");
        assert!(result.is_err());
    }

    // -------------------------------------------------------
    // Hash + Eq
    // -------------------------------------------------------
    #[test]
    fn seconds_equality() {
        let a = Seconds::from(5u64);
        let b = Seconds::from(5u64);
        let c = Seconds::from(10u64);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn seconds_hash_consistency() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Seconds::from(5u64));
        set.insert(Seconds::from(5u64));
        assert_eq!(set.len(), 1);
    }

    // -------------------------------------------------------
    // Serde deserialization via TOML (exercises visit_i64/visit_str visitors)
    // -------------------------------------------------------
    #[test]
    fn deserialize_seconds_from_toml_integer() {
        #[derive(serde::Deserialize, Debug)]
        struct W {
            val: Seconds,
        }
        let w: W = toml_edit::de::from_str("val = 42").unwrap();
        assert_eq!(*w.val, Duration::from_secs(42));
    }

    #[test]
    fn deserialize_seconds_from_toml_float() {
        #[derive(serde::Deserialize, Debug)]
        struct W {
            val: Seconds,
        }
        let w: W = toml_edit::de::from_str("val = 3.5").unwrap();
        let diff = w.val.as_secs_f64() - 3.5;
        assert!(diff.abs() < 0.01, "Expected ~3.5, got diff: {diff}");
    }

    #[test]
    fn deserialize_seconds_from_toml_string() {
        #[derive(serde::Deserialize, Debug)]
        struct W {
            val: Seconds,
        }
        let w: W = toml_edit::de::from_str(r#"val = "2.5""#).unwrap();
        assert_eq!(*w.val, Duration::from_secs_f64(2.5));
    }

    #[test]
    fn deserialize_negative_seconds_from_toml_fails() {
        #[derive(serde::Deserialize, Debug)]
        struct W {
            val: Seconds,
        }
        let result: Result<W, _> = toml_edit::de::from_str("val = -5");
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_negative_string_seconds_from_toml_fails() {
        #[derive(serde::Deserialize, Debug)]
        struct W {
            val: Seconds,
        }
        let result: Result<W, _> = toml_edit::de::from_str(r#"val = "-1""#);
        assert!(result.is_err());
    }

    #[test]
    fn deserialize_non_number_string_from_toml_fails() {
        #[derive(serde::Deserialize, Debug)]
        struct W {
            val: Seconds,
        }
        let result: Result<W, _> = toml_edit::de::from_str(r#"val = "abc""#);
        assert!(result.is_err());
    }

    // -------------------------------------------------------
    // Serde round-trip with TOML (exercises integer visitors)
    // -------------------------------------------------------
    #[test]
    fn serde_round_trip_toml_integer() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Wrapper {
            seconds: Seconds,
        }

        let original = Wrapper {
            seconds: Seconds::from(10u32),
        };
        let toml_str = toml_edit::ser::to_string_pretty(&original).unwrap();
        let restored: Wrapper = toml_edit::de::from_str(&toml_str).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn serde_round_trip_toml_float() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Wrapper {
            seconds: Seconds,
        }

        let original = Wrapper {
            seconds: Seconds::try_from(2.5f64).unwrap(),
        };
        let toml_str = toml_edit::ser::to_string_pretty(&original).unwrap();
        let restored: Wrapper = toml_edit::de::from_str(&toml_str).unwrap();
        // Floating point round-trip may not be exact, just check it's close
        let diff = restored.seconds.as_secs_f64() - original.seconds.as_secs_f64();
        assert!(diff.abs() < 0.01, "Expected close values, got diff: {diff}");
    }

    // -------------------------------------------------------
    // Error trait impls
    // -------------------------------------------------------
    #[test]
    fn parse_seconds_error_is_std_error() {
        let err: Box<dyn std::error::Error> = Box::new(ParseSecondsError::NotANumber);
        let _ = format!("{err}");
    }

    #[test]
    fn negative_seconds_is_std_error() {
        let err: Box<dyn std::error::Error> = Box::new(NegativeSeconds);
        let _ = format!("{err}");
    }

    // -------------------------------------------------------
    // Clone and Copy
    // -------------------------------------------------------
    #[test]
    fn seconds_is_copy() {
        let a = Seconds::from(5u32);
        let b = a; // Copy
        assert_eq!(a, b);
    }

    #[test]
    fn seconds_copy_produces_equal_value() {
        let a = Seconds::from(5u32);
        let b = a; // Copy
        assert_eq!(a, b);
    }

    #[test]
    fn parse_seconds_error_is_copy() {
        let a = ParseSecondsError::NotANumber;
        let b = a; // Copy
        assert_eq!(a, b);
    }
}
