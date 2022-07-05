use super::serde::{deserialize_from_str, serialize_to_str};
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt,
    ops::{Deref, DerefMut},
    str::FromStr,
};

/// Contains extra information for connections and other use cases
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extra(HashMap<String, String>);

impl Extra {
    pub fn new() -> Self {
        Self(HashMap::new())
    }
}

impl Default for Extra {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for Extra {
    type Target = HashMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Extra {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl fmt::Display for Extra {
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

impl FromStr for Extra {
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

            // Remove any extra whitespace at the front of the tail
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

impl Serialize for Extra {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_to_str(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Extra {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_from_str(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! extra {
        ($($key:literal -> $value:literal),*) => {{
            let mut _map = HashMap::new();

            $(
                _map.insert($key.to_string(), $value.to_string());
            )*

            Extra(_map)
        }};
    }

    #[test]
    fn should_support_being_parsed_from_str() {
        // Empty string (whitespace only) yields an empty map
        let extra = "   ".parse::<Extra>().unwrap();
        assert_eq!(extra, extra!());

        // Simple key=value should succeed
        let extra = "key=value".parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "value"));

        // Key can be anything but =
        let extra = "key.with-characters@=value".parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key.with-characters@" -> "value"));

        // Value can be anything but ,
        let extra = "key=value.has -@#$".parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "value.has -@#$"));

        // Value can include comma if quoted
        let extra = r#"key=",,,,""#.parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> ",,,,"));

        // Supports whitespace around key and value
        let extra = "  key  =  value  ".parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "value"));

        // Supports value capturing whitespace if quoted
        let extra = r#"  key  =   " value "   "#.parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> " value "));

        // Multiple key=value should succeed
        let extra = "key=value,key2=value2".parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "value", "key2" -> "value2"));

        // Quoted key=value should succeed
        let extra = r#"key="value one",key2=value2"#.parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "value one", "key2" -> "value2"));

        let extra = r#"key=value,key2="value two""#.parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "value", "key2" -> "value two"));

        let extra = r#"key="value one",key2="value two""#.parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "value one", "key2" -> "value two"));

        let extra = r#"key="1,2,3",key2="4,5,6""#.parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "1,2,3", "key2" -> "4,5,6"));

        // Dangling comma is okay
        let extra = "key=value,".parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "value"));
        let extra = r#"key=",value,","#.parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> ",value,"));

        // Demonstrating greedy
        let extra = "key=value key2=value2".parse::<Extra>().unwrap();
        assert_eq!(extra, extra!("key" -> "value key2=value2"));

        // Variety of edge cases that should fail
        let _ = ",".parse::<Extra>().unwrap_err();
        let _ = ",key=value".parse::<Extra>().unwrap_err();
        let _ = "key=value,key2".parse::<Extra>().unwrap_err();
    }

    #[test]
    fn should_support_being_displayed_as_a_string() {
        let extra = extra!().to_string();
        assert_eq!(extra, "");

        let extra = extra!("key" -> "value").to_string();
        assert_eq!(extra, r#"key="value""#);

        // Order of key=value output is not guaranteed
        let extra = extra!("key" -> "value", "key2" -> "value2").to_string();
        assert!(
            extra == r#"key="value",key2="value2""# || extra == r#"key2="value2",key="value""#,
            "{:?}",
            extra
        );

        // Order of key=value output is not guaranteed
        let extra = extra!("key" -> ",", "key2" -> ",,").to_string();
        assert!(
            extra == r#"key=",",key2=",,""# || extra == r#"key2=",,",key=",""#,
            "{:?}",
            extra
        );
    }
}
