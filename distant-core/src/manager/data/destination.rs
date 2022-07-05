use super::serde::{deserialize_from_str, serialize_to_str};
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::{
    convert::TryFrom,
    fmt,
    hash::Hash,
    ops::{Deref, DerefMut},
    str::FromStr,
};
use uriparse::URIReference;

/// `distant` connects and logs into the specified destination, which may be specified as either
/// `hostname:port` where an attempt to connect to a **distant** server will be made, or a URI of
/// one of the following forms:
///
/// * `distant://hostname:port` - connect to a distant server
/// * `ssh://[user@]hostname[:port]` - connect to an SSH server
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Destination(URIReference<'static>);

impl Destination {
    /// Returns true if destination represents a distant server
    pub fn is_distant(&self) -> bool {
        match self.scheme() {
            Some(scheme) => scheme.as_str().eq_ignore_ascii_case("distant"),

            // Without scheme, distant is usd by default
            None => true,
        }
    }

    /// Returns true if destination represents an ssh server
    pub fn is_ssh(&self) -> bool {
        match self.scheme() {
            Some(scheme) => scheme.as_str().eq_ignore_ascii_case("ssh"),
            None => false,
        }
    }
}

impl AsRef<Destination> for &Destination {
    fn as_ref(&self) -> &Destination {
        *self
    }
}

impl Deref for Destination {
    type Target = URIReference<'static>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Destination {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Destination {
    type Err = uriparse::URIReferenceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        URIReference::try_from(s)
            .map(URIReference::into_owned)
            .map(Destination)
    }
}

impl FromStr for Box<Destination> {
    type Err = uriparse::URIReferenceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let destination = s.parse::<Destination>()?;
        Ok(Box::new(destination))
    }
}

impl Serialize for Destination {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_to_str(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Destination {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_from_str(deserializer)
    }
}
