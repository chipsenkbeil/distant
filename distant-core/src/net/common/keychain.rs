use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::net::common::HeapSecretKey;

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

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    /// Helper to create a HeapSecretKey from a byte slice for tests.
    fn make_key(bytes: &[u8]) -> HeapSecretKey {
        HeapSecretKey::from(bytes.to_vec())
    }

    // -----------------------------------------------------------------------
    // KeychainResult helper methods
    // -----------------------------------------------------------------------

    #[test]
    fn keychain_result_is_invalid_id_returns_true_for_invalid_id() {
        let result: KeychainResult<()> = KeychainResult::InvalidId;
        assert!(result.is_invalid_id());
    }

    #[test]
    fn keychain_result_is_invalid_id_returns_false_for_invalid_password() {
        let result: KeychainResult<()> = KeychainResult::InvalidPassword;
        assert!(!result.is_invalid_id());
    }

    #[test]
    fn keychain_result_is_invalid_id_returns_false_for_ok() {
        let result = KeychainResult::Ok(42);
        assert!(!result.is_invalid_id());
    }

    #[test]
    fn keychain_result_is_invalid_password_returns_true_for_invalid_password() {
        let result: KeychainResult<()> = KeychainResult::InvalidPassword;
        assert!(result.is_invalid_password());
    }

    #[test]
    fn keychain_result_is_invalid_password_returns_false_for_invalid_id() {
        let result: KeychainResult<()> = KeychainResult::InvalidId;
        assert!(!result.is_invalid_password());
    }

    #[test]
    fn keychain_result_is_invalid_password_returns_false_for_ok() {
        let result = KeychainResult::Ok(42);
        assert!(!result.is_invalid_password());
    }

    #[test]
    fn keychain_result_is_invalid_returns_true_for_invalid_id() {
        let result: KeychainResult<()> = KeychainResult::InvalidId;
        assert!(result.is_invalid());
    }

    #[test]
    fn keychain_result_is_invalid_returns_true_for_invalid_password() {
        let result: KeychainResult<()> = KeychainResult::InvalidPassword;
        assert!(result.is_invalid());
    }

    #[test]
    fn keychain_result_is_invalid_returns_false_for_ok() {
        let result = KeychainResult::Ok(42);
        assert!(!result.is_invalid());
    }

    #[test]
    fn keychain_result_is_ok_returns_true_for_ok() {
        let result = KeychainResult::Ok(42);
        assert!(result.is_ok());
    }

    #[test]
    fn keychain_result_is_ok_returns_false_for_invalid_id() {
        let result: KeychainResult<()> = KeychainResult::InvalidId;
        assert!(!result.is_ok());
    }

    #[test]
    fn keychain_result_is_ok_returns_false_for_invalid_password() {
        let result: KeychainResult<()> = KeychainResult::InvalidPassword;
        assert!(!result.is_ok());
    }

    // -----------------------------------------------------------------------
    // KeychainResult into_ok
    // -----------------------------------------------------------------------

    #[test]
    fn keychain_result_into_ok_returns_some_for_ok() {
        let result = KeychainResult::Ok(99);
        assert_eq!(result.into_ok(), Some(99));
    }

    #[test]
    fn keychain_result_into_ok_returns_none_for_invalid_id() {
        let result: KeychainResult<i32> = KeychainResult::InvalidId;
        assert_eq!(result.into_ok(), None);
    }

    #[test]
    fn keychain_result_into_ok_returns_none_for_invalid_password() {
        let result: KeychainResult<i32> = KeychainResult::InvalidPassword;
        assert_eq!(result.into_ok(), None);
    }

    // -----------------------------------------------------------------------
    // From<KeychainResult<T>> for Option<T>
    // -----------------------------------------------------------------------

    #[test]
    fn option_from_keychain_result_ok_returns_some() {
        let result = KeychainResult::Ok(String::from("hello"));
        let opt: Option<String> = result.into();
        assert_eq!(opt, Some(String::from("hello")));
    }

    #[test]
    fn option_from_keychain_result_invalid_id_returns_none() {
        let result: KeychainResult<String> = KeychainResult::InvalidId;
        let opt: Option<String> = result.into();
        assert_eq!(opt, None);
    }

    #[test]
    fn option_from_keychain_result_invalid_password_returns_none() {
        let result: KeychainResult<String> = KeychainResult::InvalidPassword;
        let opt: Option<String> = result.into();
        assert_eq!(opt, None);
    }

    // -----------------------------------------------------------------------
    // Keychain::new creates empty keychain
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_new_is_empty() {
        let kc: Keychain<()> = Keychain::new();
        assert!(!kc.has_id("anything").await);
    }

    // -----------------------------------------------------------------------
    // Keychain insert
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_insert_returns_none_on_first_insert() {
        let kc: Keychain<i32> = Keychain::new();
        let prev = kc.insert("id1", make_key(b"key1"), 10).await;
        assert_eq!(prev, None);
    }

    #[test(tokio::test)]
    async fn keychain_insert_returns_old_data_on_overwrite() {
        let kc: Keychain<i32> = Keychain::new();
        kc.insert("id1", make_key(b"key1"), 10).await;
        let prev = kc.insert("id1", make_key(b"key2"), 20).await;
        assert_eq!(prev, Some(10));
    }

    // -----------------------------------------------------------------------
    // Keychain has_id
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_has_id_returns_false_for_missing_id() {
        let kc: Keychain<()> = Keychain::new();
        assert!(!kc.has_id("nonexistent").await);
    }

    #[test(tokio::test)]
    async fn keychain_has_id_returns_true_for_present_id() {
        let kc: Keychain<i32> = Keychain::new();
        kc.insert("id1", make_key(b"key1"), 1).await;
        assert!(kc.has_id("id1").await);
    }

    // -----------------------------------------------------------------------
    // Keychain has_key
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_has_key_returns_false_for_missing_id() {
        let kc: Keychain<i32> = Keychain::new();
        assert!(!kc.has_key("no_such_id", make_key(b"key")).await);
    }

    #[test(tokio::test)]
    async fn keychain_has_key_returns_false_for_wrong_key() {
        let kc: Keychain<i32> = Keychain::new();
        kc.insert("id1", make_key(b"correct"), 1).await;
        assert!(!kc.has_key("id1", make_key(b"wrong")).await);
    }

    #[test(tokio::test)]
    async fn keychain_has_key_returns_true_for_matching_key() {
        let kc: Keychain<i32> = Keychain::new();
        kc.insert("id1", make_key(b"correct"), 1).await;
        assert!(kc.has_key("id1", make_key(b"correct")).await);
    }

    // -----------------------------------------------------------------------
    // Keychain remove
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_remove_returns_none_for_missing_id() {
        let kc: Keychain<i32> = Keychain::new();
        assert_eq!(kc.remove("no_such_id").await, None);
    }

    #[test(tokio::test)]
    async fn keychain_remove_returns_data_and_removes_entry() {
        let kc: Keychain<i32> = Keychain::new();
        kc.insert("id1", make_key(b"key1"), 42).await;
        let removed = kc.remove("id1").await;
        assert_eq!(removed, Some(42));
        assert!(!kc.has_id("id1").await);
    }

    // -----------------------------------------------------------------------
    // Keychain remove_if_has_key
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_remove_if_has_key_returns_invalid_id_for_missing_id() {
        let kc: Keychain<i32> = Keychain::new();
        let result = kc.remove_if_has_key("no_such_id", make_key(b"key")).await;
        assert!(result.is_invalid_id());
    }

    #[test(tokio::test)]
    async fn keychain_remove_if_has_key_returns_invalid_password_for_wrong_key() {
        let kc: Keychain<i32> = Keychain::new();
        kc.insert("id1", make_key(b"correct"), 10).await;
        let result = kc.remove_if_has_key("id1", make_key(b"wrong")).await;
        assert!(result.is_invalid_password());
        // Entry should still exist.
        assert!(kc.has_id("id1").await);
    }

    #[test(tokio::test)]
    async fn keychain_remove_if_has_key_returns_ok_and_removes_for_matching_key() {
        let kc: Keychain<i32> = Keychain::new();
        kc.insert("id1", make_key(b"correct"), 10).await;
        let result = kc.remove_if_has_key("id1", make_key(b"correct")).await;
        assert_eq!(result, KeychainResult::Ok(10));
        assert!(!kc.has_id("id1").await);
    }

    // -----------------------------------------------------------------------
    // Keychain<()>::put
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_put_inserts_unit_data() {
        let kc: Keychain<()> = Keychain::new();
        kc.put("id1", make_key(b"key1")).await;
        assert!(kc.has_id("id1").await);
        assert!(kc.has_key("id1", make_key(b"key1")).await);
    }

    #[test(tokio::test)]
    async fn keychain_put_overwrites_existing_entry() {
        let kc: Keychain<()> = Keychain::new();
        kc.put("id1", make_key(b"old_key")).await;
        kc.put("id1", make_key(b"new_key")).await;
        assert!(kc.has_key("id1", make_key(b"new_key")).await);
        assert!(!kc.has_key("id1", make_key(b"old_key")).await);
    }

    // -----------------------------------------------------------------------
    // Default trait
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_default_creates_empty_keychain() {
        let kc: Keychain = Keychain::default();
        assert!(!kc.has_id("anything").await);
    }

    // -----------------------------------------------------------------------
    // From<HashMap<String, (HeapSecretKey, T)>> for Keychain<T>
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_from_hashmap_with_data() {
        let mut map = HashMap::new();
        map.insert("id1".to_string(), (make_key(b"key1"), 100));
        map.insert("id2".to_string(), (make_key(b"key2"), 200));

        let kc: Keychain<i32> = Keychain::from(map);

        assert!(kc.has_id("id1").await);
        assert!(kc.has_id("id2").await);
        assert!(kc.has_key("id1", make_key(b"key1")).await);
        assert!(kc.has_key("id2", make_key(b"key2")).await);

        let data = kc.remove("id1").await;
        assert_eq!(data, Some(100));
    }

    // -----------------------------------------------------------------------
    // From<HashMap<String, HeapSecretKey>> for Keychain<()>
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_from_hashmap_without_data() {
        let mut map = HashMap::new();
        map.insert("id1".to_string(), make_key(b"key1"));
        map.insert("id2".to_string(), make_key(b"key2"));

        let kc: Keychain<()> = Keychain::from(map);

        assert!(kc.has_id("id1").await);
        assert!(kc.has_id("id2").await);
        assert!(kc.has_key("id1", make_key(b"key1")).await);
        assert!(kc.has_key("id2", make_key(b"key2")).await);
    }

    // -----------------------------------------------------------------------
    // Clone shares underlying storage
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_clone_shares_storage() {
        let kc1: Keychain<i32> = Keychain::new();
        let kc2 = kc1.clone();

        // Insert via kc1, visible via kc2.
        kc1.insert("id1", make_key(b"key1"), 1).await;
        assert!(kc2.has_id("id1").await);
        assert!(kc2.has_key("id1", make_key(b"key1")).await);

        // Remove via kc2, no longer visible via kc1.
        kc2.remove("id1").await;
        assert!(!kc1.has_id("id1").await);
    }

    // -----------------------------------------------------------------------
    // Multiple entries
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_supports_multiple_entries() {
        let kc: Keychain<&str> = Keychain::new();
        kc.insert("a", make_key(b"ka"), "data_a").await;
        kc.insert("b", make_key(b"kb"), "data_b").await;
        kc.insert("c", make_key(b"kc"), "data_c").await;

        assert!(kc.has_id("a").await);
        assert!(kc.has_id("b").await);
        assert!(kc.has_id("c").await);

        // Remove one; others remain.
        kc.remove("b").await;
        assert!(kc.has_id("a").await);
        assert!(!kc.has_id("b").await);
        assert!(kc.has_id("c").await);
    }

    // -----------------------------------------------------------------------
    // KeychainResult Copy and Clone derive sanity
    // -----------------------------------------------------------------------

    #[test]
    fn keychain_result_can_be_copied_and_cloned() {
        let result = KeychainResult::Ok(42);
        let copied = result;
        let cloned = result;
        assert_eq!(copied, cloned);
        assert_eq!(result, KeychainResult::Ok(42));
    }

    // -----------------------------------------------------------------------
    // KeychainResult PartialEq and Debug derive sanity
    // -----------------------------------------------------------------------

    #[test]
    fn keychain_result_equality() {
        assert_eq!(
            KeychainResult::<()>::InvalidId,
            KeychainResult::<()>::InvalidId
        );
        assert_eq!(
            KeychainResult::<()>::InvalidPassword,
            KeychainResult::<()>::InvalidPassword
        );
        assert_eq!(KeychainResult::Ok(1), KeychainResult::Ok(1));
        assert_ne!(KeychainResult::Ok(1), KeychainResult::Ok(2));
        assert_ne!(
            KeychainResult::<()>::InvalidId,
            KeychainResult::<()>::InvalidPassword
        );
    }

    #[test]
    fn keychain_result_debug_format() {
        let result = KeychainResult::Ok(42);
        let debug_str = format!("{:?}", result);
        assert!(debug_str.contains("Ok"));
        assert!(debug_str.contains("42"));

        let invalid_id: KeychainResult<()> = KeychainResult::InvalidId;
        let debug_str = format!("{:?}", invalid_id);
        assert!(debug_str.contains("InvalidId"));

        let invalid_pw: KeychainResult<()> = KeychainResult::InvalidPassword;
        let debug_str = format!("{:?}", invalid_pw);
        assert!(debug_str.contains("InvalidPassword"));
    }

    // -----------------------------------------------------------------------
    // insert accepts various string types (Into<String>)
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_insert_accepts_string_and_str() {
        let kc: Keychain<()> = Keychain::new();

        // &str
        kc.insert("str_id", make_key(b"k1"), ()).await;
        assert!(kc.has_id("str_id").await);

        // String
        kc.insert(String::from("string_id"), make_key(b"k2"), ())
            .await;
        assert!(kc.has_id("string_id").await);
    }

    // -----------------------------------------------------------------------
    // remove_if_has_key does not remove entry on wrong key
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn keychain_remove_if_has_key_preserves_data_on_wrong_key() {
        let kc: Keychain<i32> = Keychain::new();
        kc.insert("id1", make_key(b"right"), 55).await;

        let result = kc.remove_if_has_key("id1", make_key(b"wrong")).await;
        assert!(result.is_invalid_password());

        // The original data should still be retrievable.
        let result = kc.remove_if_has_key("id1", make_key(b"right")).await;
        assert_eq!(result, KeychainResult::Ok(55));
    }
}
