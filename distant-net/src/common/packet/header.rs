use crate::common::Value;
use derive_more::IntoIterator;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

/// Generates a new [`Header`] of key/value pairs based on literals.
///
/// ```
/// use distant_net::header;
///
/// let _header = header!("key" -> "value", "key2" -> 123);
/// ```
#[macro_export]
macro_rules! header {
    ($($key:literal -> $value:expr),* $(,)?) => {{
        let mut _header = $crate::common::Header::default();

        $(
            _header.insert($key, $value);
        )*

        _header
    }};
}

/// Represents a packet header comprised of arbitrary data tied to string keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, IntoIterator, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Header(HashMap<String, Value>);

impl Header {
    /// Creates an empty [`Header`] newtype wrapper.
    pub fn new() -> Self {
        Self::default()
    }

    /// Exists purely to support serde serialization checks.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Inserts a key-value pair into the map.
    ///
    /// If the map did not have this key present, [`None`] is returned.
    ///
    /// If the map did have this key present, the value is updated, and the old value is returned.
    /// The key is not updated, though; this matters for types that can be `==` without being
    /// identical. See the [module-level documentation](std::collections#insert-and-complex-keys)
    /// for more.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Value>) -> Option<Value> {
        self.0.insert(key.into(), value.into())
    }
}

impl Deref for Header {
    type Target = HashMap<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Header {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
