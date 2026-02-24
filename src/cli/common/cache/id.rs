use std::convert::TryFrom;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// NOTE: This type only exists due to a bug with toml-rs where a u64 cannot be stored if its
///       value is greater than i64's max as it gets written as a negative number and then
///       fails to get read back out. To avoid this, we have a wrapper type that serializes
///       and deserializes using a string
///
/// https://github.com/alexcrichton/toml-rs/issues/256
#[derive(Copy, Clone, Debug, Default, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct CacheId<T>(T)
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display;

impl<T> CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    /// Returns the value of this storage id container
    pub fn value(self) -> T {
        self.0
    }
}

impl<T> AsRef<T> for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T> AsMut<T> for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn as_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Deref for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> fmt::Display for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<T> From<CacheId<T>> for String
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn from(id: CacheId<T>) -> Self {
        id.to_string()
    }
}

impl<T> TryFrom<String> for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    type Error = T::Err;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Ok(CacheId(s.parse()?))
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `CacheId<T>`: value access, `AsRef`/`AsMut`, `Deref`/`DerefMut`,
    //! `Display`, `From`/`TryFrom`, serde (string-based serialization), hashing,
    //! defaults, and large u64 handling (the core motivation for this type).

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // value() – consumes self and returns inner
    // -------------------------------------------------------
    #[test]
    fn value_returns_inner_u64() {
        let id = CacheId::<u64>::try_from(String::from("42")).unwrap();
        assert_eq!(id.value(), 42u64);
    }

    #[test]
    fn value_returns_inner_string() {
        let id = CacheId::<String>::try_from(String::from("hello")).unwrap();
        assert_eq!(id.value(), "hello");
    }

    // -------------------------------------------------------
    // AsRef / AsMut
    // -------------------------------------------------------
    #[test]
    fn as_ref_returns_inner_ref() {
        let id = CacheId::<u64>::try_from(String::from("123")).unwrap();
        assert_eq!(id.as_ref(), &123u64);
    }

    #[test]
    fn as_mut_returns_inner_mut() {
        let mut id = CacheId::<u64>::try_from(String::from("10")).unwrap();
        *id.as_mut() = 20;
        assert_eq!(*id.as_ref(), 20);
    }

    // -------------------------------------------------------
    // Deref / DerefMut
    // -------------------------------------------------------
    #[test]
    fn deref_gives_inner_ref() {
        let id = CacheId::<u64>::try_from(String::from("99")).unwrap();
        let val: &u64 = &id;
        assert_eq!(*val, 99);
    }

    #[test]
    fn deref_mut_gives_inner_mut() {
        let mut id = CacheId::<u64>::try_from(String::from("50")).unwrap();
        *id = 75;
        assert_eq!(*id.as_ref(), 75);
    }

    // -------------------------------------------------------
    // Display
    // -------------------------------------------------------
    #[test]
    fn display_shows_inner_value() {
        let id = CacheId::<u64>::try_from(String::from("42")).unwrap();
        assert_eq!(id.to_string(), "42");
    }

    #[test]
    fn display_string_type() {
        let id = CacheId::<String>::try_from(String::from("hello")).unwrap();
        assert_eq!(id.to_string(), "hello");
    }

    // -------------------------------------------------------
    // From<CacheId<T>> for String
    // -------------------------------------------------------
    #[test]
    fn into_string() {
        let id = CacheId::<u64>::try_from(String::from("42")).unwrap();
        let s: String = id.into();
        assert_eq!(s, "42");
    }

    // -------------------------------------------------------
    // TryFrom<String>
    // -------------------------------------------------------
    #[test]
    fn try_from_string_valid() {
        let id = CacheId::<u64>::try_from(String::from("12345")).unwrap();
        assert_eq!(*id.as_ref(), 12345);
    }

    #[test]
    fn try_from_string_invalid() {
        let result = CacheId::<u64>::try_from(String::from("not_a_number"));
        assert!(result.is_err());
    }

    #[test]
    fn try_from_string_empty() {
        let result = CacheId::<u64>::try_from(String::from(""));
        assert!(result.is_err());
    }

    // -------------------------------------------------------
    // Serialization: serializes as a String
    // -------------------------------------------------------
    #[test]
    fn serialize_to_json_string() {
        let id = CacheId::<u64>::try_from(String::from("42")).unwrap();
        let json = serde_json::to_string(&id).unwrap();
        // It should serialize as "42" (a JSON string), not as 42 (a JSON number)
        assert_eq!(json, r#""42""#);
    }

    #[test]
    fn deserialize_from_json_string() {
        let id: CacheId<u64> = serde_json::from_str(r#""99""#).unwrap();
        assert_eq!(*id.as_ref(), 99);
    }

    #[test]
    fn serde_round_trip() {
        let original = CacheId::<u64>::try_from(String::from("12345678")).unwrap();
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: CacheId<u64> = serde_json::from_str(&json).unwrap();
        assert_eq!(*deserialized.as_ref(), 12345678);
    }

    // -------------------------------------------------------
    // Large u64 values (the reason CacheId exists)
    // -------------------------------------------------------
    #[test]
    fn handles_large_u64_values() {
        // Values > i64::MAX were the original motivation for CacheId
        let large_val = u64::MAX;
        let id = CacheId::<u64>::try_from(large_val.to_string()).unwrap();
        assert_eq!(*id.as_ref(), u64::MAX);
    }

    #[test]
    fn large_u64_serde_round_trip() {
        let large_val = u64::MAX;
        let id = CacheId::<u64>::try_from(large_val.to_string()).unwrap();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: CacheId<u64> = serde_json::from_str(&json).unwrap();
        assert_eq!(*deserialized.as_ref(), u64::MAX);
    }

    #[test]
    fn handles_value_just_above_i64_max() {
        let val = (i64::MAX as u64) + 1;
        let id = CacheId::<u64>::try_from(val.to_string()).unwrap();
        assert_eq!(*id.as_ref(), val);
    }

    // -------------------------------------------------------
    // Copy
    // -------------------------------------------------------
    #[test]
    fn copy_produces_equal_display() {
        // CacheId<u64> is Copy, so assignment copies rather than moves.
        let id = CacheId::<u64>::try_from(String::from("42")).unwrap();
        let copied = id; // Copy, not Clone
        assert_eq!(id.to_string(), copied.to_string());
    }

    // -------------------------------------------------------
    // Default
    // -------------------------------------------------------
    #[test]
    fn default_u64_is_zero() {
        let id = CacheId::<u64>::default();
        assert_eq!(*id.as_ref(), 0);
    }

    #[test]
    fn default_string_is_empty() {
        let id = CacheId::<String>::default();
        assert_eq!(id.as_ref(), "");
    }

    // -------------------------------------------------------
    // Hash – use HashMap with String keys from Display
    // -------------------------------------------------------
    #[test]
    fn display_values_match_for_equal_inner() {
        let id1 = CacheId::<u64>::try_from(String::from("42")).unwrap();
        let id2 = CacheId::<u64>::try_from(String::from("42")).unwrap();
        let id3 = CacheId::<u64>::try_from(String::from("99")).unwrap();
        // Verify Display values are the same for equal inner values
        assert_eq!(id1.to_string(), id2.to_string());
        assert_ne!(id1.to_string(), id3.to_string());
    }

    #[test]
    fn hash_via_display_key_deduplicates() {
        use std::collections::HashMap;
        let id1 = CacheId::<u64>::try_from(String::from("42")).unwrap();
        let id2 = CacheId::<u64>::try_from(String::from("42")).unwrap();
        let mut map = HashMap::new();
        map.insert(id1.to_string(), "first");
        map.insert(id2.to_string(), "second");
        // Same key, so should overwrite
        assert_eq!(map.len(), 1);
        assert_eq!(map["42"], "second");
    }

    // -------------------------------------------------------
    // With String inner type – full lifecycle
    // -------------------------------------------------------
    #[test]
    fn cache_id_string_lifecycle() {
        let id = CacheId::<String>::try_from(String::from("my-key")).unwrap();
        assert_eq!(id.to_string(), "my-key");
        assert_eq!(id.as_ref(), "my-key");

        let json = serde_json::to_string(&id).unwrap();
        let deserialized: CacheId<String> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.to_string(), "my-key");
    }

    #[test]
    fn cache_id_string_value_consumes() {
        let id = CacheId::<String>::try_from(String::from("consumed")).unwrap();
        let val = id.value();
        assert_eq!(val, "consumed");
    }
}
