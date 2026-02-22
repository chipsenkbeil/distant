use std::ops::{Deref, DerefMut};
use std::str::FromStr;

use derive_more::{Display, IsVariant};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Represents a value for some CLI option or config. This exists to support optional values that
/// have a default value so we can distinguish if a CLI value was a default or explicitly defined.
#[derive(Copy, Clone, Debug, Display, IsVariant)]
pub enum Value<T> {
    /// Value is a default representation.
    Default(T),

    /// Value is explicitly defined by the user.
    Explicit(T),
}

impl<T> Value<T> {
    pub fn into_inner(self) -> T {
        match self {
            Self::Default(x) => x,
            Self::Explicit(x) => x,
        }
    }
}

impl<T> AsRef<T> for Value<T> {
    fn as_ref(&self) -> &T {
        match self {
            Value::Default(x) => x,
            Value::Explicit(x) => x,
        }
    }
}

impl<T> AsMut<T> for Value<T> {
    fn as_mut(&mut self) -> &mut T {
        match self {
            Value::Default(x) => x,
            Value::Explicit(x) => x,
        }
    }
}

impl<T> Deref for Value<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        AsRef::as_ref(self)
    }
}

impl<T> DerefMut for Value<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        AsMut::as_mut(self)
    }
}
/*
impl<T> Into<T> for Value<T> {
    fn into(self) -> T {
        match self {
            Self::Default(x) => x,
            Self::Explicit(x) => x,
        }
    }
} */

impl<T> PartialEq for Value<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        AsRef::as_ref(self) == AsRef::as_ref(other)
    }
}

impl<T> PartialEq<T> for Value<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &T) -> bool {
        AsRef::as_ref(self) == other
    }
}

impl<T> FromStr for Value<T>
where
    T: FromStr,
{
    type Err = T::Err;

    /// Parses `s` into [Value], placing the result into the explicit variant.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::Explicit(T::from_str(s)?))
    }
}

impl<T> Serialize for Value<T>
where
    T: Serialize,
{
    /// Serializes the underlying data within [Value]. The origin of the value (default vs
    /// explicit) is not stored as config files using serialization are all explicitly set.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        T::serialize(self, serializer)
    }
}

impl<'de, T> Deserialize<'de> for Value<T>
where
    T: Deserialize<'de>,
{
    /// Deserializes into an explicit variant of [Value]. It is assumed that any value coming from
    /// a format like a config.toml is explicitly defined and not a default, even though we have a
    /// default config.toml available.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::Explicit(T::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // into_inner
    // -------------------------------------------------------
    #[test]
    fn into_inner_default() {
        let v = Value::Default(42);
        assert_eq!(v.into_inner(), 42);
    }

    #[test]
    fn into_inner_explicit() {
        let v = Value::Explicit(99);
        assert_eq!(v.into_inner(), 99);
    }

    // -------------------------------------------------------
    // is_default / is_explicit
    // -------------------------------------------------------
    #[test]
    fn is_default_returns_true_for_default() {
        let v = Value::Default(1);
        assert!(v.is_default());
        assert!(!v.is_explicit());
    }

    #[test]
    fn is_explicit_returns_true_for_explicit() {
        let v = Value::Explicit(1);
        assert!(v.is_explicit());
        assert!(!v.is_default());
    }

    // -------------------------------------------------------
    // AsRef
    // -------------------------------------------------------
    #[test]
    fn as_ref_default() {
        let v = Value::Default(42);
        assert_eq!(v.as_ref(), &42);
    }

    #[test]
    fn as_ref_explicit() {
        let v = Value::Explicit(42);
        assert_eq!(v.as_ref(), &42);
    }

    // -------------------------------------------------------
    // AsMut
    // -------------------------------------------------------
    #[test]
    fn as_mut_default() {
        let mut v = Value::Default(42);
        *v.as_mut() = 100;
        assert_eq!(v.into_inner(), 100);
    }

    #[test]
    fn as_mut_explicit() {
        let mut v = Value::Explicit(42);
        *v.as_mut() = 100;
        assert_eq!(v.into_inner(), 100);
    }

    // -------------------------------------------------------
    // Deref / DerefMut
    // -------------------------------------------------------
    #[test]
    fn deref_accesses_inner() {
        let v = Value::Default(String::from("hello"));
        // Deref to &String, call len()
        assert_eq!(v.len(), 5);
    }

    #[test]
    fn deref_mut_modifies_inner() {
        let mut v = Value::Default(String::from("hello"));
        v.push_str(" world");
        assert_eq!(&*v, "hello world");
    }

    // -------------------------------------------------------
    // PartialEq<Value<T>>
    // -------------------------------------------------------
    #[test]
    fn partial_eq_same_variant_same_value() {
        assert_eq!(Value::Default(5), Value::Default(5));
        assert_eq!(Value::Explicit(5), Value::Explicit(5));
    }

    #[test]
    fn partial_eq_different_variant_same_value() {
        // Default(5) == Explicit(5) because we compare inner values
        assert_eq!(Value::Default(5), Value::Explicit(5));
    }

    #[test]
    fn partial_eq_different_values() {
        assert_ne!(Value::Default(5), Value::Default(10));
        assert_ne!(Value::Explicit(5), Value::Explicit(10));
    }

    // -------------------------------------------------------
    // PartialEq<T>
    // -------------------------------------------------------
    #[test]
    fn partial_eq_with_inner_type() {
        let v = Value::Default(42);
        assert_eq!(v, 42);
        assert_ne!(v, 99);
    }

    #[test]
    fn partial_eq_explicit_with_inner_type() {
        let v = Value::Explicit(42);
        assert_eq!(v, 42);
    }

    // -------------------------------------------------------
    // FromStr
    // -------------------------------------------------------
    #[test]
    fn from_str_produces_explicit_variant() {
        let v: Value<i32> = "42".parse().unwrap();
        assert!(v.is_explicit());
        assert_eq!(v.into_inner(), 42);
    }

    #[test]
    fn from_str_error_propagated() {
        let result = "not_a_number".parse::<Value<i32>>();
        assert!(result.is_err());
    }

    #[test]
    fn from_str_string_type() {
        // String::from_str never fails
        let v: Value<String> = "hello".parse().unwrap();
        assert!(v.is_explicit());
        assert_eq!(&*v, "hello");
    }

    // -------------------------------------------------------
    // Display
    // -------------------------------------------------------
    #[test]
    fn display_default() {
        let v = Value::Default(42);
        assert_eq!(v.to_string(), "42");
    }

    #[test]
    fn display_explicit() {
        let v = Value::Explicit(42);
        assert_eq!(v.to_string(), "42");
    }

    // -------------------------------------------------------
    // Serialize
    // -------------------------------------------------------
    #[test]
    fn serialize_default_value() {
        let v = Value::Default(42);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "42");
    }

    #[test]
    fn serialize_explicit_value() {
        let v = Value::Explicit(42);
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "42");
    }

    #[test]
    fn serialize_string_value() {
        let v = Value::Explicit(String::from("hello"));
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#""hello""#);
    }

    // -------------------------------------------------------
    // Deserialize
    // -------------------------------------------------------
    #[test]
    fn deserialize_produces_explicit_variant() {
        let v: Value<i32> = serde_json::from_str("42").unwrap();
        assert!(v.is_explicit());
        assert_eq!(v.into_inner(), 42);
    }

    #[test]
    fn deserialize_string() {
        let v: Value<String> = serde_json::from_str(r#""world""#).unwrap();
        assert!(v.is_explicit());
        assert_eq!(&*v, "world");
    }

    // -------------------------------------------------------
    // round-trip serialize/deserialize
    // -------------------------------------------------------
    #[test]
    fn serde_round_trip() {
        let original = Value::Default(123);
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Value<i32> = serde_json::from_str(&json).unwrap();
        // After round-trip, it becomes Explicit (from deserialization)
        assert!(deserialized.is_explicit());
        assert_eq!(deserialized, original); // PartialEq compares inner values
    }

    // -------------------------------------------------------
    // Clone
    // -------------------------------------------------------
    #[test]
    fn clone_preserves_variant() {
        let v = Value::Default(42);
        let cloned = v;
        assert!(cloned.is_default());
        assert_eq!(cloned.into_inner(), 42);

        let v = Value::Explicit(99);
        let cloned = v;
        assert!(cloned.is_explicit());
        assert_eq!(cloned.into_inner(), 99);
    }
}
