use serde::{Deserialize, Serialize};
use std::{
    convert::TryFrom,
    fmt,
    ops::{Deref, DerefMut},
    str::FromStr,
};

/// NOTE: This type only exists due to a bug with toml-rs where a u64 cannot be stored if its
///       value is greater than i64's max as it gets written as a negative number and then
///       fails to get read back out. To avoid this, we have a wrapper type that serializes
///       and deserializes using a string
///
/// https://github.com/alexcrichton/toml-rs/issues/256
#[derive(Copy, Clone, Debug, Default, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct StorageId<T>(T)
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display;

impl<T> StorageId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    /// Returns the value of this storage id container
    pub fn value(self) -> T {
        self.0
    }
}

impl<T> AsRef<T> for StorageId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T> AsMut<T> for StorageId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn as_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Deref for StorageId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for StorageId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> fmt::Display for StorageId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<T> From<StorageId<T>> for String
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    fn from(id: StorageId<T>) -> Self {
        id.to_string()
    }
}

impl<T> TryFrom<String> for StorageId<T>
where
    T: fmt::Display + FromStr + Clone,
    T::Err: fmt::Display,
{
    type Error = T::Err;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Ok(StorageId(s.parse()?))
    }
}
