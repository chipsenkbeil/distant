use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

use serde::de::{Deserializer, Error as SerdeError, Visitor};
use serde::ser::Serializer;

/// From https://docs.rs/serde_with/1.14.0/src/serde_with/rust.rs.html#90-118
pub fn deserialize_from_str<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    struct Helper<S>(PhantomData<S>);

    impl<'de, S> Visitor<'de> for Helper<S>
    where
        S: FromStr,
        <S as FromStr>::Err: fmt::Display,
    {
        type Value = S;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "a string")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: SerdeError,
        {
            value.parse::<Self::Value>().map_err(SerdeError::custom)
        }
    }

    deserializer.deserialize_str(Helper(PhantomData))
}

/// From https://docs.rs/serde_with/1.14.0/src/serde_with/rust.rs.html#121-127
pub fn serialize_to_str<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: fmt::Display,
    S: Serializer,
{
    serializer.collect_str(&value)
}

#[cfg(test)]
mod tests {
    //! Tests for deserialize_from_str and serialize_to_str serde helpers: round-trips,
    //! string-not-number serialization format, parse error handling, and edge cases
    //! (zero, max u32, empty string, leading whitespace).

    use serde::{Deserialize, Serialize};
    use test_log::test;

    /// Test wrapper struct that uses the serde helper functions to serialize/deserialize
    /// a `u32` value as a string.
    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct TestWrapper {
        #[serde(
            deserialize_with = "super::deserialize_from_str",
            serialize_with = "super::serialize_to_str"
        )]
        value: u32,
    }

    /// A wrapper around a signed integer to test with a different type.
    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct SignedWrapper {
        #[serde(
            deserialize_with = "super::deserialize_from_str",
            serialize_with = "super::serialize_to_str"
        )]
        value: i64,
    }

    #[test]
    fn serialize_then_deserialize_round_trip() {
        let original = TestWrapper { value: 42 };
        let json = serde_json::to_string(&original).expect("serialize failed");
        let restored: TestWrapper = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(original, restored);
    }

    #[test]
    fn serialize_produces_string_value_not_number() {
        let wrapper = TestWrapper { value: 12345 };
        let json = serde_json::to_string(&wrapper).expect("serialize failed");
        // The value should be serialized as the string "12345", not the number 12345
        assert_eq!(json, r#"{"value":"12345"}"#);
    }

    #[test]
    fn deserialize_valid_string_value() {
        let json = r#"{"value":"99"}"#;
        let wrapper: TestWrapper = serde_json::from_str(json).expect("deserialize failed");
        assert_eq!(wrapper.value, 99);
    }

    #[test]
    fn deserialize_invalid_string_fails() {
        let json = r#"{"value":"not_a_number"}"#;
        let result: Result<TestWrapper, _> = serde_json::from_str(json);
        assert!(result.is_err(), "expected error for invalid string value");
    }

    #[test]
    fn deserialize_number_instead_of_string_fails() {
        // Since deserialize_from_str expects a string, passing a raw number should fail
        let json = r#"{"value":42}"#;
        let result: Result<TestWrapper, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "expected error when JSON value is a number instead of a string"
        );
    }

    #[test]
    fn serialize_zero_value() {
        let wrapper = TestWrapper { value: 0 };
        let json = serde_json::to_string(&wrapper).expect("serialize failed");
        assert_eq!(json, r#"{"value":"0"}"#);
    }

    #[test]
    fn deserialize_zero_string() {
        let json = r#"{"value":"0"}"#;
        let wrapper: TestWrapper = serde_json::from_str(json).expect("deserialize failed");
        assert_eq!(wrapper.value, 0);
    }

    #[test]
    fn round_trip_with_signed_integer() {
        let original = SignedWrapper { value: -42 };
        let json = serde_json::to_string(&original).expect("serialize failed");
        assert_eq!(json, r#"{"value":"-42"}"#);
        let restored: SignedWrapper = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(original, restored);
    }

    #[test]
    fn round_trip_with_max_u32_value() {
        let original = TestWrapper { value: u32::MAX };
        let json = serde_json::to_string(&original).expect("serialize failed");
        assert_eq!(json, r#"{"value":"4294967295"}"#);
        let restored: TestWrapper = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(original, restored);
    }

    #[test]
    fn deserialize_empty_string_fails() {
        let json = r#"{"value":""}"#;
        let result: Result<TestWrapper, _> = serde_json::from_str(json);
        assert!(result.is_err(), "expected error for empty string");
    }

    #[test]
    fn deserialize_string_with_leading_whitespace_fails() {
        let json = r#"{"value":" 42"}"#;
        let result: Result<TestWrapper, _> = serde_json::from_str(json);
        // FromStr for u32 does not accept leading whitespace
        assert!(
            result.is_err(),
            "expected error for string with leading whitespace"
        );
    }
}
