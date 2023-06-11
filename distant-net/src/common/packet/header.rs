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
        let mut _header = ::std::collections::HashMap::new();

        $(
            _header.insert($key.to_string(), $crate::common::Value::from($value));
        )*

        $crate::common::Header::new(_header)
    }};
}

/// Represents a packet header for a request or response
#[derive(Clone, Debug, Default, PartialEq, Eq, IntoIterator, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Header(HashMap<String, Value>);

impl Header {
    /// Creates a new [`Header`] newtype wrapper.
    pub fn new(map: HashMap<String, Value>) -> Self {
        Self(map)
    }

    /// Exists purely to support serde serialization checks.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
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
