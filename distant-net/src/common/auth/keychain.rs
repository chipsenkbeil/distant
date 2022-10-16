use crate::common::HeapSecretKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Manages keys with associated ids. Cloning will result in a copy pointing to the same underlying
/// storage, which enables support of managing the keys across multiple threads.
#[derive(Clone, Debug)]
pub struct Keychain {
    map: Arc<RwLock<HashMap<String, HeapSecretKey>>>,
}

impl Keychain {
    /// Creates a new keychain without any keys.
    pub fn new() -> Self {
        Self {
            map: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Stores a new `key` by a given `id`, returning the old key if there was one already
    /// registered.
    pub async fn insert(&self, id: impl Into<String>, key: HeapSecretKey) -> Option<HeapSecretKey> {
        self.map.write().await.insert(id.into(), key)
    }

    /// Checks if there is a key with the given `id` that matches the provided `key`.
    pub async fn has_key(&self, id: impl AsRef<str>, key: impl PartialEq<HeapSecretKey>) -> bool {
        self.map
            .read()
            .await
            .get(id.as_ref())
            .map(|k| key.eq(k))
            .unwrap_or(false)
    }

    /// Removes a key by a given `id`, returning the key if there was one found for the given id.
    pub async fn remove(&self, id: impl AsRef<str>) -> Option<HeapSecretKey> {
        self.map.write().await.remove(id.as_ref())
    }
}

impl Default for Keychain {
    fn default() -> Self {
        Self::new()
    }
}

impl From<HashMap<String, HeapSecretKey>> for Keychain {
    /// Creates a new keychain populated with the provided `map`.
    fn from(map: HashMap<String, HeapSecretKey>) -> Self {
        Self {
            map: Arc::new(RwLock::new(map)),
        }
    }
}
