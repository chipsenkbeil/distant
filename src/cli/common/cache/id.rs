use std::convert::TryFrom;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// NOTE: This type only exists due to a bug with toml-rs where a u64 cannot be stored if its
///       value is greater than i64's max as it gets written as a negative number and then
///       fails to get read back out. To avoid this, we have a wrapper type that serializes
///       and deserializes using a string
///
/// https://github.com/alexcrichton/toml-rs/issues/256
#[derive(Copy, Clone, Debug, Default, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct CacheId<T>(T)
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display;

impl<T> CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    /// Returns the value of this storage id container
    pub fn value(self) -> T {
        self.0
    }
}

impl<T> AsRef<T> for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T> AsMut<T> for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn as_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Deref for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> fmt::Display for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<T> From<CacheId<T>> for String
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn from(id: CacheId<T>) -> Self {
        id.to_string()
    }
}

impl<T> TryFrom<String> for CacheId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    type Error = T::Err;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Ok(CacheId(s.parse()?))
    }
}
