use crate::common::HeapSecretKey;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Represents the result of a request to the database.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum KeychainResult<T> {
    /// Id was not found in the database.
    InvalidId,

    /// Password match for an id failed.
    InvalidPassword,

    /// Successful match of id and password, removing from keychain and returning data `T`.
    Ok(T),
}

impl<T> KeychainResult<T> {
    pub fn is_invalid_id(&self) -> bool {
        matches!(self, Self::InvalidId)
    }

    pub fn is_invalid_password(&self) -> bool {
        matches!(self, Self::InvalidPassword)
    }

    pub fn is_invalid(&self) -> bool {
        matches!(self, Self::InvalidId | Self::InvalidPassword)
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }

    pub fn into_ok(self) -> Option<T> {
        match self {
            Self::Ok(x) => Some(x),
            _ => None,
        }
    }
}

impl<T> From<KeychainResult<T>> for Option<T> {
    fn from(result: KeychainResult<T>) -> Self {
        result.into_ok()
    }
}

/// Manages keys with associated ids. Cloning will result in a copy pointing to the same underlying
/// storage, which enables support of managing the keys across multiple threads.
#[derive(Debug)]
pub struct Keychain<T = ()> {
    map: Arc<RwLock<HashMap<String, (HeapSecretKey, T)>>>,
}

impl<T> Clone for Keychain<T> {
    fn clone(&self) -> Self {
        Self {
            map: Arc::clone(&self.map),
        }
    }
}

impl<T> Keychain<T> {
    /// Creates a new keychain without any keys.
    pub fn new() -> Self {
        Self {
            map: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Stores a new `key` and `data` by a given `id`, returning the old data associated with the
    /// id if there was one already registered.
    pub async fn insert(&self, id: impl Into<String>, key: HeapSecretKey, data: T) -> Option<T> {
        self.map
            .write()
            .await
            .insert(id.into(), (key, data))
            .map(|(_, data)| data)
    }

    /// Checks if there is an `id` stored within the keychain.
    pub async fn has_id(&self, id: impl AsRef<str>) -> bool {
        self.map.read().await.contains_key(id.as_ref())
    }

    /// Checks if there is a key with the given `id` that matches the provided `key`.
    pub async fn has_key(&self, id: impl AsRef<str>, key: impl PartialEq<HeapSecretKey>) -> bool {
        self.map
            .read()
            .await
            .get(id.as_ref())
            .map(|(k, _)| key.eq(k))
            .unwrap_or(false)
    }

    /// Removes a key and its data by a given `id`, returning the data if the `id` exists.
    pub async fn remove(&self, id: impl AsRef<str>) -> Option<T> {
        self.map
            .write()
            .await
            .remove(id.as_ref())
            .map(|(_, data)| data)
    }

    /// Checks if there is a key with the given `id` that matches the provided `key`, returning the
    /// data if the `id` exists and the `key` matches.
    pub async fn remove_if_has_key(
        &self,
        id: impl AsRef<str>,
        key: impl PartialEq<HeapSecretKey>,
    ) -> KeychainResult<T> {
        let id = id.as_ref();
        let mut lock = self.map.write().await;

        match lock.get(id) {
            Some((k, _)) if key.eq(k) => KeychainResult::Ok(lock.remove(id).unwrap().1),
            Some(_) => KeychainResult::InvalidPassword,
            None => KeychainResult::InvalidId,
        }
    }
}

impl Keychain<()> {
    /// Stores a new `key by a given `id`.
    pub async fn put(&self, id: impl Into<String>, key: HeapSecretKey) {
        self.insert(id, key, ()).await;
    }
}

impl Default for Keychain {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> From<HashMap<String, (HeapSecretKey, T)>> for Keychain<T> {
    /// Creates a new keychain populated with the provided `map`.
    fn from(map: HashMap<String, (HeapSecretKey, T)>) -> Self {
        Self {
            map: Arc::new(RwLock::new(map)),
        }
    }
}

impl From<HashMap<String, HeapSecretKey>> for Keychain<()> {
    /// Creates a new keychain populated with the provided `map`.
    fn from(map: HashMap<String, HeapSecretKey>) -> Self {
        Self::from(
            map.into_iter()
                .map(|(id, key)| (id, (key, ())))
                .collect::<HashMap<String, (HeapSecretKey, ())>>(),
        )
    }
}
