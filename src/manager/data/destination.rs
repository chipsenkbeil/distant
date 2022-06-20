use serde::{
    de::{Deserializer, Error as SerdeError, Visitor},
    ser::Serializer,
    Deserialize, Serialize,
};
use std::{
    convert::TryFrom,
    fmt,
    hash::Hash,
    marker::PhantomData,
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

/// From https://docs.rs/serde_with/1.14.0/src/serde_with/rust.rs.html#90-118
fn deserialize_from_str<'de, D, T>(deserializer: D) -> Result<T, D::Error>
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
fn serialize_to_str<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: fmt::Display,
    S: Serializer,
{
    serializer.collect_str(&value)
}
