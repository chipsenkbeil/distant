use crate::common::utils;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::io;
use std::ops::{Deref, DerefMut};

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
