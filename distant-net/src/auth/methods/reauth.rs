use super::{AuthenticationMethod, Authenticator, Challenge, Error, Question};
use crate::HeapSecretKey;
use async_trait::async_trait;
use std::collections::HashMap;
use std::io;
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

/// Authenticaton method for reauthentication, which supports authentication using a mapping of
/// some id to an associated secret key. The method uses a generic [`Keychain`] in order to manage
/// the keys that can be used.
#[derive(Clone, Debug)]
pub struct ReauthenticationMethod {
    keychain: Keychain,
}

impl ReauthenticationMethod {
    pub fn new(keychain: Keychain) -> Self {
        Self { keychain }
    }
}

#[async_trait]
impl AuthenticationMethod for ReauthenticationMethod {
    fn id(&self) -> &'static str {
        "reauthentication"
    }

    async fn authenticate(&self, authenticator: &mut dyn Authenticator) -> io::Result<()> {
        let response = authenticator
            .challenge(Challenge {
                questions: vec![Question::new("id"), Question::new("key")],
                options: Default::default(),
            })
            .await?;

        if response.answers.len() != 2 {
            return Err(Error::non_fatal("wrong answer count").into_io_permission_denied());
        }

        if self
            .keychain
            .has_key(
                &response.answers[0],
                response.answers[1].parse::<HeapSecretKey>()?,
            )
            .await
        {
            Ok(())
        } else {
            Err(Error::non_fatal("invalid id").into_io_permission_denied())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        auth::msg::{AuthenticationResponse, ChallengeResponse},
        utils, FramedTransport,
    };
    use test_log::test;

    /// Creates a test keychain with a single mapping of "id" -> "secret key".
    #[inline]
    fn new_keychain() -> Keychain {
        Keychain::from(
            vec![(
                "id".to_string(),
                HeapSecretKey::from(b"secret key".to_vec()),
            )]
            .into_iter()
            .collect::<HashMap<_, _>>(),
        )
    }

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_challenge_fails() {
        let method = ReauthenticationMethod::new(new_keychain());
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up an invalid frame for our challenge to ensure it fails
        t2.write_frame(b"invalid initialization response")
            .await
            .unwrap();

        assert_eq!(
            method.authenticate(&mut t1).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_answer_is_not_exactly_an_id_and_key() {
        let method = ReauthenticationMethod::new(new_keychain());
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame(
            utils::serialize_to_vec(&AuthenticationResponse::Challenge(ChallengeResponse {
                answers: vec!["id".to_string()],
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(
            method.authenticate(&mut t1).await.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_answer_key_is_invalid() {
        let method = ReauthenticationMethod::new(new_keychain());
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame(
            utils::serialize_to_vec(&AuthenticationResponse::Challenge(ChallengeResponse {
                answers: vec!["id".to_string(), "secret key".to_string()],
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(
            method.authenticate(&mut t1).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_answer_provides_invalid_id() {
        let method = ReauthenticationMethod::new(new_keychain());
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame(
            utils::serialize_to_vec(&AuthenticationResponse::Challenge(ChallengeResponse {
                answers: vec![
                    "id2".to_string(),
                    HeapSecretKey::from(b"secret key".to_vec()).to_string(),
                ],
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(
            method.authenticate(&mut t1).await.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_answer_provides_wrong_key() {
        let method = ReauthenticationMethod::new(new_keychain());
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame(
            utils::serialize_to_vec(&AuthenticationResponse::Challenge(ChallengeResponse {
                answers: vec![
                    "id".to_string(),
                    HeapSecretKey::from(b"wrong secret key".to_vec()).to_string(),
                ],
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(
            method.authenticate(&mut t1).await.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test(tokio::test)]
    async fn authenticate_should_succeed_if_challenge_response_is_valid_id_and_key() {
        let method = ReauthenticationMethod::new(new_keychain());
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame(
            utils::serialize_to_vec(&AuthenticationResponse::Challenge(ChallengeResponse {
                answers: vec![
                    "id".to_string(),
                    HeapSecretKey::from(b"secret key".to_vec()).to_string(),
                ],
            }))
            .unwrap(),
        )
        .await
        .unwrap();

        method.authenticate(&mut t1).await.unwrap();
    }
}
