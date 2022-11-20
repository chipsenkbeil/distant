use crate::common::utils::{deserialize_from_str, serialize_to_str};
use derive_more::{Display, Error, From, IntoIterator};
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt,
    ops::{Deref, DerefMut},
    str::FromStr,
};

/// Contains map information for connections and other use cases
#[derive(Clone, Debug, From, IntoIterator, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Map(HashMap<String, String>);

impl Map {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn into_map(self) -> HashMap<String, String> {
        self.0
    }
}

#[cfg(feature = "schemars")]
impl Map {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Map)
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
            write!(f, "{}=\"{}\"", key, encode_value(value))?;

            // Include a comma after each but the last pair
            if i + 1 < len {
                write!(f, ",")?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Display, Error)]
pub enum MapParseError {
    #[display(fmt = "Missing = after key ('{}')", key)]
    MissingEqualsAfterKey { key: String },

    #[display(fmt = "Key ('{}') must start with alphabetic character", key)]
    KeyMustStartWithAlphabeticCharacter { key: String },

    #[display(fmt = "Missing closing \" for value")]
    MissingClosingQuoteForValue,
}

impl FromStr for Map {
    type Err = MapParseError;

    /// Parses a series of `key=value` pairs in the form `key="value",key2=value2` where
    /// the quotes around the value are optional
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut map = HashMap::new();

        let mut s = s.trim();
        while !s.is_empty() {
            // Find {key}={tail...} where tail is everything after =
            let (key, tail) = s
                .split_once('=')
                .ok_or_else(|| MapParseError::MissingEqualsAfterKey { key: s.to_string() })?;

            // Remove whitespace around the key and ensure it starts with a proper character
            let key = key.trim();

            if !key.starts_with(char::is_alphabetic) {
                return Err(MapParseError::KeyMustStartWithAlphabeticCharacter {
                    key: key.to_string(),
                });
            }

            // Remove any map whitespace at the front of the tail
            let tail = tail.trim_start();

            // Determine if we start with a quote " otherwise we will look for the next ,
            let (value, tail) = match tail.strip_prefix('"') {
                // If quoted, we maintain the whitespace within the quotes
                Some(tail) => {
                    let mut backslash_cnt: usize = 0;
                    let mut split_idx = None;
                    for (i, b) in tail.bytes().enumerate() {
                        // If we get a backslash, increment our count
                        if b == b'\\' {
                            backslash_cnt += 1;

                        // If we get a quote and have an even number of preceding backslashes,
                        // this is considered a closing quote and we've found our split index
                        } else if b == b'"' && backslash_cnt % 2 == 0 {
                            split_idx = Some(i);
                            break;

                        // Otherwise, we've encountered some other character, so reset our backlash
                        // count
                        } else {
                            backslash_cnt = 0;
                        }
                    }

                    match split_idx {
                        Some(i) => {
                            // Splitting at idx will result in the double quote being at the
                            // beginning of the tail str, so we want to have the tail start
                            // one after the beginning of the slice
                            let (value, tail) = tail.split_at(i);
                            let tail = &tail[1..].trim_start();

                            // Also remove a trailing comma if it exists
                            let tail = tail.strip_prefix(',').unwrap_or(tail).trim_start();

                            (value, tail)
                        }
                        None => return Err(MapParseError::MissingClosingQuoteForValue),
                    }
                }

                // If not quoted, we remove all whitespace around the value
                None => match tail.split_once(',') {
                    Some((value, tail)) => (value.trim(), tail),
                    None => (tail.trim(), ""),
                },
            };

            // Insert our new pair and update the slice to be the tail (removing whitespace)
            map.insert(key.to_string(), decode_value(value));
            s = tail.trim();
        }

        Ok(Self(map))
    }
}

/// Escapes double-quotes of a value str
/// * `\` -> `\\`
/// * `"` -> `\"`
#[inline]
fn encode_value(value: &str) -> String {
    // \ -> \\
    // " -> \"
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Translates escaped double-quotes back into quotes
/// * `\\` -> `\`
/// * `\"` -> `"`
#[inline]
fn decode_value(value: &str) -> String {
    value.replace("\\\\", "\\").replace("\\\"", "\"")
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

/// Generates a new [`Map`] of key/value pairs based on literals.
///
/// ```
/// use distant_net::map;
///
/// let _map = map!("key" -> "value", "key2" -> "value2");
/// ```
#[macro_export]
macro_rules! map {
    ($($key:literal -> $value:literal),* $(,)?) => {{
        let mut _map = ::std::collections::HashMap::new();

        $(
            _map.insert($key.to_string(), $value.to_string());
        )*

        $crate::common::Map::from(_map)
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

        // Support escaped quotes within value
        let map = r#"key="\"va\"lue\"""#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> r#""va"lue""#));

        // Support escaped backslashes within value
        let map = r#"key="a\\b\\c""#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> r"a\b\c"));

        // Backslashes and double quotes in a value are escaped together
        let map = r#"key="\"\\\"\\\"""#.parse::<Map>().unwrap();
        assert_eq!(map, map!("key" -> r#""\"\""#));

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

        // Double quotes in a value are escaped
        let map = map!("key" -> r#""va"lue""#).to_string();
        assert_eq!(map, r#"key="\"va\"lue\"""#);

        // Backslashes in a value are also escaped
        let map = map!("key" -> r"a\b\c").to_string();
        assert_eq!(map, r#"key="a\\b\\c""#);

        // Backslashes and double quotes in a value are escaped together
        let map = map!("key" -> r#""\"\""#).to_string();
        assert_eq!(map, r#"key="\"\\\"\\\"""#);

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
