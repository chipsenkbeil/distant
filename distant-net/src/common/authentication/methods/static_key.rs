use std::io;

use async_trait::async_trait;

use super::{AuthenticationMethod, Authenticator, Challenge, Error, Question};
use crate::common::HeapSecretKey;

/// Authenticaton method for a static secret key
#[derive(Clone, Debug)]
pub struct StaticKeyAuthenticationMethod {
    key: HeapSecretKey,
}

impl StaticKeyAuthenticationMethod {
    #[inline]
    pub fn new(key: impl Into<HeapSecretKey>) -> Self {
        Self { key: key.into() }
    }
}

#[async_trait]
impl AuthenticationMethod for StaticKeyAuthenticationMethod {
    fn id(&self) -> &'static str {
        "static_key"
    }

    async fn authenticate(&self, authenticator: &mut dyn Authenticator) -> io::Result<()> {
        let response = authenticator
            .challenge(Challenge {
                questions: vec![Question {
                    label: "key".to_string(),
                    text: "Provide a key: ".to_string(),
                    options: Default::default(),
                }],
                options: Default::default(),
            })
            .await?;

        if response.answers.is_empty() {
            return Err(Error::non_fatal("missing answer").into_io_permission_denied());
        }

        match response
            .answers
            .into_iter()
            .next()
            .unwrap()
            .parse::<HeapSecretKey>()
        {
            Ok(key) if key == self.key => Ok(()),
            _ => Err(Error::non_fatal("answer does not match key").into_io_permission_denied()),
        }
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;
    use crate::common::authentication::msg::{AuthenticationResponse, ChallengeResponse};
    use crate::common::FramedTransport;

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_key_challenge_fails() {
        let method = StaticKeyAuthenticationMethod::new(b"".to_vec());
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
    async fn authenticate_should_fail_if_no_answer_included_in_challenge_response() {
        let method = StaticKeyAuthenticationMethod::new(b"".to_vec());
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame_for(&AuthenticationResponse::Challenge(ChallengeResponse {
            answers: Vec::new(),
        }))
        .await
        .unwrap();

        assert_eq!(
            method.authenticate(&mut t1).await.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_answer_does_not_match_key() {
        let method = StaticKeyAuthenticationMethod::new(b"answer".to_vec());
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame_for(&AuthenticationResponse::Challenge(ChallengeResponse {
            answers: vec![HeapSecretKey::from(b"some key".to_vec()).to_string()],
        }))
        .await
        .unwrap();

        assert_eq!(
            method.authenticate(&mut t1).await.unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test(tokio::test)]
    async fn authenticate_should_succeed_if_answer_matches_key() {
        let method = StaticKeyAuthenticationMethod::new(b"answer".to_vec());
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame_for(&AuthenticationResponse::Challenge(ChallengeResponse {
            answers: vec![HeapSecretKey::from(b"answer".to_vec()).to_string()],
        }))
        .await
        .unwrap();

        method.authenticate(&mut t1).await.unwrap();
    }
}
