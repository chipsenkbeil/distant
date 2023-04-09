use std::ops::{Deref, DerefMut};
use std::str::FromStr;

/// Represents a value for some CLI option or configuration file. This exists to support optional
/// values that have a default value so we can distinguish if a CLI value was a default or
/// explicitly defined.
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
