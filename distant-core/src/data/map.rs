use crate::serde_str::{deserialize_from_str, serialize_to_str};
use derive_more::{From, IntoIterator};
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt,
    ops::{Deref, DerefMut},
    str::FromStr,
};

/// Contains map information for connections and other use cases
#[derive(Clone, Debug, From, IntoIterator, PartialEq, Eq)]
pub struct Map(HashMap<String, String>);

impl Map {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn into_map(self) -> HashMap<String, String> {
        self.0
    }
}

impl Default for Map {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for Map {
    type Target = HashMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Map {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl fmt::Display for Map {
    /// Outputs a `key=value` mapping in the form `key="value",key2="value2"`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = self.0.len();
        for (i, (key, value)) in self.0.iter().enumerate() {
            write!(f, "{}=\"{}\"", key, value)?;

            // Include a comma after each but the last pair
            if i + 1 < len {
                write!(f, ",")?;
            }
        }
        Ok(())
    }
}

impl FromStr for Map {
    type Err = &'static str;

    /// Parses a series of `key=value` pairs in the form `key="value",key2=value2` where
    /// the quotes around the value are optional
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut map = HashMap::new();

        let mut s = s.trim();
        while !s.is_empty() {
            // Find {key}={tail...} where tail is everything after =
            let (key, tail) = s.split_once('=').ok_or("Missing = after key")?;

            // Remove whitespace around the key and ensure it starts with a proper character
            let key = key.trim();

            if !key.starts_with(char::is_alphabetic) {
                return Err("Key must start with alphabetic character");
            }

            // Remove any map whitespace at the front of the tail
            let tail = tail.trim_start();

            // Determine if we start with a quote " otherwise we will look for the next ,
            let (value, tail) = match tail.strip_prefix('"') {
                // If quoted, we maintain the whitespace within the quotes
                Some(tail) => {
                    // Skip the quote so we can look for the trailing quote
                    let (value, tail) =
                        tail.split_once('"').ok_or("Missing closing \" for value")?;

                    // Skip comma if we have one
                    let tail = tail.strip_prefix(',').unwrap_or(tail);

                    (value, tail)
                }

                // If not quoted, we remove all whitespace around the value
                None => match tail.split_once(',') {
                    Some((value, tail)) => (value.trim(), tail),
                    None => (tail.trim(), ""),
                },
            };

            // Insert our new pair and update the slice to be the tail (removing whitespace)
            map.insert(key.to_string(), value.to_string());
            s = tail.trim();
        }

        Ok(Self(map))
    }
}

impl Serialize for Map {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_to_str(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Map {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_from_str(deserializer)
    }
}

#[macro_export]
macro_rules! map {
    ($($key:literal -> $value:literal),*) => {{
        let mut _map = ::std::collections::HashMap::new();

        $(
            _map.insert($key.to_string(), $value.to_string());
        )*

        $crate::Map::from(_map)
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_support_being_parsed_from_str() {
        // Empty string (whitespace only) yields an empty map
        let map = "   ".parse::<Map>().unwrap();
        assert_eq!(map, map!());

        // Simple key=value should succeed
        let map = "key=value".parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "value"));

        // Key can be anything but =
        let map = "key.with-characters@=value".parse::<Map>().unwrap();
        assert_eq!(map, map!("key.with-characters@" -> "value"));

        // Value can be anything but ,
        let map = "key=value.has -@#$".parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "value.has -@#$"));

        // Value can include comma if quoted
        let map = r#"key=",,,,""#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> ",,,,"));

        // Supports whitespace around key and value
        let map = "  key  =  value  ".parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "value"));

        // Supports value capturing whitespace if quoted
        let map = r#"  key  =   " value "   "#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> " value "));

        // Multiple key=value should succeed
        let map = "key=value,key2=value2".parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "value", "key2" -> "value2"));

        // Quoted key=value should succeed
        let map = r#"key="value one",key2=value2"#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "value one", "key2" -> "value2"));

        let map = r#"key=value,key2="value two""#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "value", "key2" -> "value two"));

        let map = r#"key="value one",key2="value two""#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "value one", "key2" -> "value two"));

        let map = r#"key="1,2,3",key2="4,5,6""#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "1,2,3", "key2" -> "4,5,6"));

        // Dangling comma is okay
        let map = "key=value,".parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "value"));
        let map = r#"key=",value,","#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> ",value,"));

        // Demonstrating greedy
        let map = "key=value key2=value2".parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> "value key2=value2"));

        // Variety of edge cases that should fail
        let _ = ",".parse::<Map>().unwrap_err();
        let _ = ",key=value".parse::<Map>().unwrap_err();
        let _ = "key=value,key2".parse::<Map>().unwrap_err();
    }

    #[test]
    fn should_support_being_displayed_as_a_string() {
        let map = map!().to_string();
        assert_eq!(map, "");

        let map = map!("key" -> "value").to_string();
        assert_eq!(map, r#"key="value""#);

        // Order of key=value output is not guaranteed
        let map = map!("key" -> "value", "key2" -> "value2").to_string();
        assert!(
            map == r#"key="value",key2="value2""# || map == r#"key2="value2",key="value""#,
            "{:?}",
            map
        );

        // Order of key=value output is not guaranteed
        let map = map!("key" -> ",", "key2" -> ",,").to_string();
        assert!(
            map == r#"key=",",key2=",,""# || map == r#"key2=",,",key=",""#,
            "{:?}",
            map
        );
    }
}
