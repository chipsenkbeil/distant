use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::{fmt, io};

use derive_more::IntoIterator;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::net::common::{Value, utils};

/// Generates a new [`Header`] of key/value pairs based on literals.
///
/// ```
/// use distant_core::header;
///
/// let _header = header!("key" -> "value", "key2" -> 123);
/// ```
#[macro_export]
macro_rules! header {
    ($($key:literal -> $value:expr),* $(,)?) => {{
        let mut _header = $crate::net::common::Header::default();

        $(
            _header.insert($key, $value);
        )*

        _header
    }};
}

/// Represents a packet header comprised of arbitrary data tied to string keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, IntoIterator, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Header(HashMap<String, Value>);

impl Header {
    /// Creates an empty [`Header`] newtype wrapper.
    pub fn new() -> Self {
        Self::default()
    }

    /// Exists purely to support serde serialization checks.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Inserts a key-value pair into the map.
    ///
    /// If the map did not have this key present, [`None`] is returned.
    ///
    /// If the map did have this key present, the value is updated, and the old value is returned.
    /// The key is not updated, though; this matters for types that can be `==` without being
    /// identical. See the [module-level documentation](std::collections#insert-and-complex-keys)
    /// for more.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) -> Option<Value> {
        self.0.insert(key.into(), value.into())
    }

    /// Retrieves a value from the header, attempting to convert it to the specified type `T`
    /// by cloning the value and then converting it.
    pub fn get_as<T>(&self, key: impl AsRef<str>) -> Option<io::Result<T>>
    where
        T: DeserializeOwned,
    {
        self.0
            .get(key.as_ref())
            .map(|value| value.clone().cast_as())
    }

    /// Serializes the header into bytes.
    pub fn to_vec(&self) -> io::Result<Vec<u8>> {
        utils::serialize_to_vec(self)
    }

    /// Deserializes the header from bytes.
    pub fn from_slice(slice: &[u8]) -> io::Result<Self> {
        utils::deserialize_from_slice(slice)
    }
}

impl Deref for Header {
    type Target = HashMap<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Header {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl fmt::Display for Header {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{")?;

        for (key, value) in self.0.iter() {
            let value = serde_json::to_string(value).unwrap_or_else(|_| String::from("--"));
            write!(f, "\"{key}\" = {value}")?;
        }

        write!(f, "}}")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Tests for Header: construction, insert/retrieve, msgpack and JSON round-trips,
    //! Display formatting, Deref/DerefMut delegation, header! macro, equality, clone,
    //! and iteration.

    use super::*;

    mod new_and_is_empty {
        use test_log::test;

        use super::*;

        #[test]
        fn new_should_create_empty_header() {
            let header = Header::new();
            assert!(header.is_empty());
        }

        #[test]
        fn default_should_create_empty_header() {
            let header = Header::default();
            assert!(header.is_empty());
        }

        #[test]
        fn is_empty_should_return_false_after_insert() {
            let mut header = Header::new();
            header.insert("key", "value");
            assert!(!header.is_empty());
        }
    }

    mod insert_and_get_as {
        use test_log::test;

        use super::*;

        #[test]
        fn should_insert_and_retrieve_string_value() {
            let mut header = Header::new();
            header.insert("name", "alice");
            let result: String = header.get_as("name").unwrap().unwrap();
            assert_eq!(result, "alice");
        }

        #[test]
        fn should_insert_and_retrieve_integer_value() {
            let mut header = Header::new();
            header.insert("count", 42);
            let result: i64 = header.get_as("count").unwrap().unwrap();
            assert_eq!(result, 42);
        }

        #[test]
        fn should_insert_and_retrieve_bool_value() {
            let mut header = Header::new();
            header.insert("flag", true);
            let result: bool = header.get_as("flag").unwrap().unwrap();
            assert!(result);
        }

        #[test]
        fn should_insert_and_retrieve_unsigned_integer() {
            let mut header = Header::new();
            header.insert("big", 999u64);
            let result: u64 = header.get_as("big").unwrap().unwrap();
            assert_eq!(result, 999);
        }

        #[test]
        fn should_return_none_for_missing_key() {
            let header = Header::new();
            let result: Option<io::Result<String>> = header.get_as("nonexistent");
            assert!(result.is_none());
        }

        #[test]
        fn should_return_error_for_wrong_type_cast() {
            let mut header = Header::new();
            header.insert("name", "not_a_number");
            let result: io::Result<i64> = header.get_as("name").unwrap();
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        }

        #[test]
        fn should_return_error_for_bool_cast_to_string() {
            let mut header = Header::new();
            header.insert("flag", true);
            let result: io::Result<String> = header.get_as("flag").unwrap();
            assert!(result.is_err());
        }

        #[test]
        fn should_return_old_value_when_inserting_duplicate_key() {
            let mut header = Header::new();
            let old = header.insert("key", "first");
            assert!(old.is_none());

            let old = header.insert("key", "second");
            assert!(old.is_some());
            let old_val: String = old.unwrap().cast_as().unwrap();
            assert_eq!(old_val, "first");

            // Current value should be the new one
            let current: String = header.get_as("key").unwrap().unwrap();
            assert_eq!(current, "second");
        }
    }

    mod to_vec_and_from_slice {
        use test_log::test;

        use super::*;

        #[test]
        fn should_round_trip_empty_header() {
            let original = Header::new();
            let bytes = original.to_vec().unwrap();
            let restored = Header::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
            assert!(restored.is_empty());
        }

        #[test]
        fn should_round_trip_header_with_string_value() {
            let mut original = Header::new();
            original.insert("greeting", "hello");
            let bytes = original.to_vec().unwrap();
            let restored = Header::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
            let val: String = restored.get_as("greeting").unwrap().unwrap();
            assert_eq!(val, "hello");
        }

        #[test]
        fn should_round_trip_header_with_number_value() {
            let mut original = Header::new();
            original.insert("count", 123);
            let bytes = original.to_vec().unwrap();
            let restored = Header::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
        }

        #[test]
        fn should_round_trip_header_with_bool_value() {
            let mut original = Header::new();
            original.insert("enabled", true);
            let bytes = original.to_vec().unwrap();
            let restored = Header::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
        }

        #[test]
        fn should_round_trip_header_with_multiple_values() {
            let mut original = Header::new();
            original.insert("name", "test");
            original.insert("version", 2);
            original.insert("debug", false);
            let bytes = original.to_vec().unwrap();
            let restored = Header::from_slice(&bytes).unwrap();
            assert_eq!(original, restored);
        }

        #[test]
        fn should_fail_to_deserialize_from_invalid_bytes() {
            let result = Header::from_slice(&[0xc1]); // Reserved msgpack byte
            assert!(result.is_err());
        }

        #[test]
        fn should_fail_to_deserialize_from_empty_slice() {
            let result = Header::from_slice(&[]);
            assert!(result.is_err());
        }
    }

    mod display {
        use test_log::test;

        use super::*;

        #[test]
        fn should_display_empty_header_as_braces() {
            let header = Header::new();
            let display = format!("{header}");
            assert_eq!(display, "{}");
        }

        #[test]
        fn should_display_single_entry_header() {
            let mut header = Header::new();
            header.insert("key", "value");
            let display = format!("{header}");
            // The format is {"key" = "value"}
            assert!(display.starts_with('{'));
            assert!(display.ends_with('}'));
            assert!(display.contains("\"key\" = \"value\""));
        }

        #[test]
        fn should_display_numeric_value() {
            let mut header = Header::new();
            header.insert("num", 42);
            let display = format!("{header}");
            assert!(display.contains("\"num\" = 42"));
        }

        #[test]
        fn should_display_bool_value() {
            let mut header = Header::new();
            header.insert("flag", true);
            let display = format!("{header}");
            assert!(display.contains("\"flag\" = true"));
        }
    }

    mod deref {
        use test_log::test;

        use super::*;

        #[test]
        fn should_expose_hashmap_len_via_deref() {
            let mut header = Header::new();
            assert_eq!(header.len(), 0);
            header.insert("a", 1);
            assert_eq!(header.len(), 1);
            header.insert("b", 2);
            assert_eq!(header.len(), 2);
        }

        #[test]
        fn should_expose_contains_key_via_deref() {
            let mut header = Header::new();
            header.insert("present", "yes");
            assert!(header.contains_key("present"));
            assert!(!header.contains_key("absent"));
        }

        #[test]
        fn should_expose_keys_via_deref() {
            let mut header = Header::new();
            header.insert("alpha", 1);
            header.insert("beta", 2);
            let keys: Vec<&String> = header.keys().collect();
            assert_eq!(keys.len(), 2);
            assert!(header.contains_key("alpha"));
            assert!(header.contains_key("beta"));
        }

        #[test]
        fn should_expose_values_via_deref() {
            let mut header = Header::new();
            header.insert("x", 10);
            let values: Vec<&Value> = header.values().collect();
            assert_eq!(values.len(), 1);
        }

        #[test]
        fn should_allow_remove_via_deref_mut() {
            let mut header = Header::new();
            header.insert("key", "value");
            assert!(header.contains_key("key"));
            header.remove("key");
            assert!(!header.contains_key("key"));
            assert!(header.is_empty());
        }

        #[test]
        fn should_allow_clear_via_deref_mut() {
            let mut header = Header::new();
            header.insert("a", 1);
            header.insert("b", 2);
            header.clear();
            assert!(header.is_empty());
        }
    }

    mod header_macro {
        use test_log::test;

        #[test]
        fn should_create_empty_header() {
            let h = header!();
            assert!(h.is_empty());
        }

        #[test]
        fn should_create_header_with_single_string_entry() {
            let h = header!("name" -> "alice");
            assert_eq!(h.len(), 1);
            let val: String = h.get_as("name").unwrap().unwrap();
            assert_eq!(val, "alice");
        }

        #[test]
        fn should_create_header_with_single_number_entry() {
            let h = header!("count" -> 42);
            let val: i64 = h.get_as("count").unwrap().unwrap();
            assert_eq!(val, 42);
        }

        #[test]
        fn should_create_header_with_multiple_entries() {
            let h = header!("key" -> "value", "num" -> 123, "flag" -> true);
            assert_eq!(h.len(), 3);
            let key_val: String = h.get_as("key").unwrap().unwrap();
            assert_eq!(key_val, "value");
            let num_val: i64 = h.get_as("num").unwrap().unwrap();
            assert_eq!(num_val, 123);
            let flag_val: bool = h.get_as("flag").unwrap().unwrap();
            assert!(flag_val);
        }

        #[test]
        fn should_allow_trailing_comma() {
            let h = header!("a" -> 1, "b" -> 2,);
            assert_eq!(h.len(), 2);
        }

        #[test]
        fn should_overwrite_duplicate_keys() {
            // The macro calls insert sequentially, so duplicate keys
            // result in the last value winning
            let h = header!("key" -> "first", "key" -> "second");
            assert_eq!(h.len(), 1);
            let val: String = h.get_as("key").unwrap().unwrap();
            assert_eq!(val, "second");
        }
    }

    mod serde {
        use test_log::test;

        use super::*;

        #[test]
        fn should_serialize_and_deserialize_via_serde_json() {
            let mut original = Header::new();
            original.insert("key", "value");
            original.insert("num", 42);

            let json = serde_json::to_string(&original).unwrap();
            let restored: Header = serde_json::from_str(&json).unwrap();
            assert_eq!(original, restored);
        }

        #[test]
        fn should_serialize_empty_header_as_empty_object() {
            let header = Header::new();
            let json = serde_json::to_string(&header).unwrap();
            assert_eq!(json, "{}");
        }

        #[test]
        fn should_deserialize_from_json_object() {
            let json = r#"{"name":"bob","age":30}"#;
            let header: Header = serde_json::from_str(json).unwrap();
            let name: String = header.get_as("name").unwrap().unwrap();
            assert_eq!(name, "bob");
            let age: i64 = header.get_as("age").unwrap().unwrap();
            assert_eq!(age, 30);
        }
    }

    mod equality_and_clone {
        use test_log::test;

        use super::*;

        #[test]
        fn should_be_equal_for_same_contents() {
            let mut h1 = Header::new();
            h1.insert("a", 1);
            let mut h2 = Header::new();
            h2.insert("a", 1);
            assert_eq!(h1, h2);
        }

        #[test]
        fn should_not_be_equal_for_different_contents() {
            let mut h1 = Header::new();
            h1.insert("a", 1);
            let mut h2 = Header::new();
            h2.insert("a", 2);
            assert_ne!(h1, h2);
        }

        #[test]
        fn should_clone_independently() {
            let mut original = Header::new();
            original.insert("key", "value");
            let mut cloned = original.clone();
            cloned.insert("key", "changed");

            let orig_val: String = original.get_as("key").unwrap().unwrap();
            let clone_val: String = cloned.get_as("key").unwrap().unwrap();
            assert_eq!(orig_val, "value");
            assert_eq!(clone_val, "changed");
        }
    }

    mod into_iterator {
        use test_log::test;

        #[test]
        fn should_iterate_over_key_value_pairs() {
            let h = header!("x" -> 10, "y" -> 20);
            let mut count = 0;
            for (_key, _value) in h {
                count += 1;
            }
            assert_eq!(count, 2);
        }
    }
}
