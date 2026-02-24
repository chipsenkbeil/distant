use std::borrow::Cow;
use std::io;
use std::ops::{Deref, DerefMut};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::net::common::utils;

/// Generic value type for data passed through header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Value(serde_json::Value);

impl Value {
    /// Creates a new [`Value`] by converting `value` to the underlying type.
    pub fn new(value: impl Into<serde_json::Value>) -> Self {
        Self(value.into())
    }

    /// Serializes the value into bytes.
    pub fn to_vec(&self) -> io::Result<Vec<u8>> {
        utils::serialize_to_vec(self)
    }

    /// Deserializes the value from bytes.
    pub fn from_slice(slice: &[u8]) -> io::Result<Self> {
        utils::deserialize_from_slice(slice)
    }

    /// Attempts to convert this generic value to a specific type.
    pub fn cast_as<T>(self) -> io::Result<T>
    where
        T: DeserializeOwned,
    {
        serde_json::from_value(self.0).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }
}

impl Deref for Value {
    type Target = serde_json::Value;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Value {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

macro_rules! impl_from {
    ($($type:ty),+) => {
        $(
            impl From<$type> for Value {
                fn from(x: $type) -> Self {
                    Self(From::from(x))
                }
            }
        )+
    };
}

impl_from!(
    (),
    i8, i16, i32, i64, isize,
    u8, u16, u32, u64, usize,
    f32, f64,
    bool, String, serde_json::Number,
    serde_json::Map<String, serde_json::Value>
);

impl<'a, T> From<&'a [T]> for Value
where
    T: Clone + Into<serde_json::Value>,
{
    fn from(x: &'a [T]) -> Self {
        Self(From::from(x))
    }
}

impl<'a> From<&'a str> for Value {
    fn from(x: &'a str) -> Self {
        Self(From::from(x))
    }
}

impl<'a> From<Cow<'a, str>> for Value {
    fn from(x: Cow<'a, str>) -> Self {
        Self(From::from(x))
    }
}

impl<T> From<Option<T>> for Value
where
    T: Into<serde_json::Value>,
{
    fn from(x: Option<T>) -> Self {
        Self(From::from(x))
    }
}

impl<T> From<Vec<T>> for Value
where
    T: Into<serde_json::Value>,
{
    fn from(x: Vec<T>) -> Self {
        Self(From::from(x))
    }
}

#[cfg(test)]
mod tests {
    //! Tests for Value: construction from various types, msgpack round-trips, cast_as
    //! conversions, Deref/DerefMut delegation, and From impls for all supported primitive types.

    use super::*;

    mod new {
        use test_log::test;

        use super::*;

        #[test]
        fn should_create_value_from_string() {
            let value = Value::new("hello");
            assert!(value.is_string());
            assert_eq!(value.as_str(), Some("hello"));
        }

        #[test]
        fn should_create_value_from_number() {
            let value = Value::new(42);
            assert!(value.is_number());
            assert_eq!(value.as_i64(), Some(42));
        }

        #[test]
        fn should_create_value_from_bool() {
            let value = Value::new(true);
            assert!(value.is_boolean());
            assert_eq!(value.as_bool(), Some(true));
        }

        #[test]
        fn should_create_value_from_null() {
            let value = Value::new(serde_json::Value::Null);
            assert!(value.is_null());
        }

        #[test]
        fn should_create_value_from_array() {
            let value = Value::new(serde_json::json!([1, 2, 3]));
            assert!(value.is_array());
            assert_eq!(value.as_array().unwrap().len(), 3);
        }

        #[test]
        fn should_create_value_from_object() {
            let value = Value::new(serde_json::json!({"key": "val"}));
            assert!(value.is_object());
        }
    }

    mod to_vec_and_from_slice {
        use test_log::test;

        use super::*;

        #[test]
        fn should_round_trip_string_value() {
            let original = Value::new("hello world");
            let bytes = original.to_vec().unwrap();
            let restored = Value::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
        }

        #[test]
        fn should_round_trip_integer_value() {
            let original = Value::new(12345);
            let bytes = original.to_vec().unwrap();
            let restored = Value::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
        }

        #[test]
        fn should_round_trip_bool_value() {
            let original = Value::new(false);
            let bytes = original.to_vec().unwrap();
            let restored = Value::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
        }

        #[test]
        fn should_round_trip_null_value() {
            let original = Value::new(serde_json::Value::Null);
            let bytes = original.to_vec().unwrap();
            let restored = Value::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
        }

        #[test]
        fn should_round_trip_array_value() {
            let original = Value::new(serde_json::json!([1, "two", true, null]));
            let bytes = original.to_vec().unwrap();
            let restored = Value::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
        }

        #[test]
        fn should_fail_to_deserialize_from_invalid_bytes() {
            let result = Value::from_slice(&[0xc1]); // Reserved msgpack byte
            assert!(result.is_err());
        }

        #[test]
        fn should_fail_to_deserialize_from_empty_slice() {
            let result = Value::from_slice(&[]);
            assert!(result.is_err());
        }
    }

    mod cast_as {
        use test_log::test;

        use super::*;

        #[test]
        fn should_cast_string_value_to_string() {
            let value = Value::new("hello");
            let result: String = value.cast_as().unwrap();
            assert_eq!(result, "hello");
        }

        #[test]
        fn should_cast_integer_value_to_i64() {
            let value = Value::new(42);
            let result: i64 = value.cast_as().unwrap();
            assert_eq!(result, 42);
        }

        #[test]
        fn should_cast_bool_value_to_bool() {
            let value = Value::new(true);
            let result: bool = value.cast_as().unwrap();
            assert!(result);
        }

        #[test]
        fn should_cast_array_value_to_vec() {
            let value = Value::new(serde_json::json!([1, 2, 3]));
            let result: Vec<i64> = value.cast_as().unwrap();
            assert_eq!(result, vec![1, 2, 3]);
        }

        #[test]
        fn should_fail_to_cast_string_value_to_i64() {
            let value = Value::new("not a number");
            let result: io::Result<i64> = value.cast_as();
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        }

        #[test]
        fn should_fail_to_cast_bool_value_to_string() {
            let value = Value::new(true);
            let result: io::Result<String> = value.cast_as();
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        }

        #[test]
        fn should_cast_null_to_option_none() {
            let value = Value::new(serde_json::Value::Null);
            let result: Option<String> = value.cast_as().unwrap();
            assert_eq!(result, None);
        }

        #[test]
        fn should_cast_string_to_option_some() {
            let value = Value::new("hello");
            let result: Option<String> = value.cast_as().unwrap();
            assert_eq!(result, Some("hello".to_string()));
        }
    }

    mod deref {
        use test_log::test;

        use super::*;

        #[test]
        fn should_allow_serde_json_value_methods_via_deref() {
            let value = Value::new("hello");
            // These methods come from serde_json::Value via Deref
            assert!(value.is_string());
            assert!(!value.is_number());
            assert_eq!(value.as_str(), Some("hello"));
        }

        #[test]
        fn should_allow_mutation_via_deref_mut() {
            let mut value = Value::new(serde_json::json!({"key": "old"}));
            // Access via DerefMut to mutate
            value
                .as_object_mut()
                .unwrap()
                .insert("key".to_string(), serde_json::json!("new"));
            assert_eq!(value.as_object().unwrap()["key"], serde_json::json!("new"));
        }
    }

    mod from_impls {
        use std::borrow::Cow;

        use test_log::test;

        use super::*;

        #[test]
        fn should_convert_unit_to_null_value() {
            let value = Value::from(());
            assert!(value.is_null());
        }

        #[test]
        fn should_convert_i8() {
            let value = Value::from(-42i8);
            assert_eq!(value.as_i64(), Some(-42));
        }

        #[test]
        fn should_convert_i16() {
            let value = Value::from(-1000i16);
            assert_eq!(value.as_i64(), Some(-1000));
        }

        #[test]
        fn should_convert_i32() {
            let value = Value::from(100_000i32);
            assert_eq!(value.as_i64(), Some(100_000));
        }

        #[test]
        fn should_convert_i64() {
            let value = Value::from(9_000_000_000i64);
            assert_eq!(value.as_i64(), Some(9_000_000_000));
        }

        #[test]
        fn should_convert_isize() {
            let value = Value::from(-999isize);
            assert_eq!(value.as_i64(), Some(-999));
        }

        #[test]
        fn should_convert_u8() {
            let value = Value::from(255u8);
            assert_eq!(value.as_u64(), Some(255));
        }

        #[test]
        fn should_convert_u16() {
            let value = Value::from(65535u16);
            assert_eq!(value.as_u64(), Some(65535));
        }

        #[test]
        fn should_convert_u32() {
            let value = Value::from(4_000_000_000u32);
            assert_eq!(value.as_u64(), Some(4_000_000_000));
        }

        #[test]
        fn should_convert_u64() {
            let value = Value::from(10_000_000_000u64);
            assert_eq!(value.as_u64(), Some(10_000_000_000));
        }

        #[test]
        fn should_convert_usize() {
            let value = Value::from(12345usize);
            assert_eq!(value.as_u64(), Some(12345));
        }

        #[test]
        fn should_convert_f32() {
            let value = Value::from(3.25f32);
            assert!(value.is_f64());
            // f32 -> f64 conversion may lose precision, so check approximate
            let f = value.as_f64().unwrap();
            assert!((f - 3.25).abs() < 0.01);
        }

        #[test]
        fn should_convert_f64() {
            let value = Value::from(2.5f64);
            assert!(value.is_f64());
            assert!((value.as_f64().unwrap() - 2.5).abs() < 1e-9);
        }

        #[test]
        fn should_convert_bool_true() {
            let value = Value::from(true);
            assert_eq!(value.as_bool(), Some(true));
        }

        #[test]
        fn should_convert_bool_false() {
            let value = Value::from(false);
            assert_eq!(value.as_bool(), Some(false));
        }

        #[test]
        fn should_convert_string() {
            let value = Value::from(String::from("hello"));
            assert_eq!(value.as_str(), Some("hello"));
        }

        #[test]
        fn should_convert_str_ref() {
            let value = Value::from("world");
            assert_eq!(value.as_str(), Some("world"));
        }

        #[test]
        fn should_convert_cow_borrowed() {
            let value = Value::from(Cow::Borrowed("borrowed"));
            assert_eq!(value.as_str(), Some("borrowed"));
        }

        #[test]
        fn should_convert_cow_owned() {
            let value = Value::from(Cow::<str>::Owned(String::from("owned")));
            assert_eq!(value.as_str(), Some("owned"));
        }

        #[test]
        fn should_convert_serde_json_number() {
            let num = serde_json::Number::from(99);
            let value = Value::from(num);
            assert_eq!(value.as_i64(), Some(99));
        }

        #[test]
        fn should_convert_serde_json_map() {
            let mut map = serde_json::Map::new();
            map.insert("a".to_string(), serde_json::json!(1));
            let value = Value::from(map);
            assert!(value.is_object());
            assert_eq!(value.as_object().unwrap()["a"], serde_json::json!(1));
        }

        #[test]
        fn should_convert_option_some() {
            let value = Value::from(Some(42));
            assert_eq!(value.as_i64(), Some(42));
        }

        #[test]
        fn should_convert_option_none() {
            let value = Value::from(None::<i32>);
            assert!(value.is_null());
        }

        #[test]
        fn should_convert_vec() {
            let value = Value::from(vec![1, 2, 3]);
            assert!(value.is_array());
            let arr = value.as_array().unwrap();
            assert_eq!(arr.len(), 3);
            assert_eq!(arr[0], serde_json::json!(1));
            assert_eq!(arr[1], serde_json::json!(2));
            assert_eq!(arr[2], serde_json::json!(3));
        }

        #[test]
        fn should_convert_slice() {
            let items = [1, 2, 3];
            let value = Value::from(&items[..]);
            assert!(value.is_array());
            assert_eq!(value.as_array().unwrap().len(), 3);
        }
    }
}
