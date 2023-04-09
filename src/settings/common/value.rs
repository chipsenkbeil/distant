use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

/// Represents a value for some CLI option or config. This exists to support optional values that
/// have a default value so we can distinguish if a CLI value was a default or explicitly defined.
#[derive(Copy, Clone, Debug)]
pub enum Value<T> {
    /// Value is a default representation.
    Default(T),

    /// Value is explicitly defined by the user.
    Explicit(T),
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

impl<T> Into<T> for Value<T> {
    fn into(self) -> T {
        match self {
            Self::Default(x) => x,
            Self::Explicit(x) => x,
        }
    }
}

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
