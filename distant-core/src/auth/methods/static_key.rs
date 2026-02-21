use std::io;
use std::str::FromStr;

use async_trait::async_trait;

use crate::auth::authenticator::Authenticator;
use crate::auth::methods::AuthenticationMethod;
use crate::auth::msg::{Challenge, Error, Question};

/// Authenticaton method for a static secret key
#[derive(Clone, Debug)]
pub struct StaticKeyAuthenticationMethod<T> {
    key: T,
}

impl<T> StaticKeyAuthenticationMethod<T> {
    pub const ID: &str = "static_key";

    #[inline]
    pub fn new(key: T) -> Self {
        Self { key }
    }
}

#[async_trait]
impl<T> AuthenticationMethod for StaticKeyAuthenticationMethod<T>
where
    T: FromStr + PartialEq + Send + Sync,
{
    fn id(&self) -> &'static str {
        Self::ID
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

        match response.answers.into_iter().next().unwrap().parse::<T>() {
            Ok(key) if key == self.key => Ok(()),
            _ => Err(Error::non_fatal("answer does not match key").into_io_permission_denied()),
        }
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;
    use crate::auth::authenticator::TestAuthenticator;
    use crate::auth::msg::*;

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_key_challenge_fails() {
        let method = StaticKeyAuthenticationMethod::new(String::new());

        let mut authenticator = TestAuthenticator {
            challenge: Box::new(|_| Err(io::Error::new(io::ErrorKind::InvalidData, "test error"))),
            ..Default::default()
        };

        let err = method.authenticate(&mut authenticator).await.unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(err.to_string(), "test error");
    }

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_no_answer_included_in_challenge_response() {
        let method = StaticKeyAuthenticationMethod::new(String::new());

        let mut authenticator = TestAuthenticator {
            challenge: Box::new(|_| {
                Ok(ChallengeResponse {
                    answers: Vec::new(),
                })
            }),
            ..Default::default()
        };

        let err = method.authenticate(&mut authenticator).await.unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(err.to_string(), "Error: missing answer");
    }

    #[test(tokio::test)]
    async fn authenticate_should_fail_if_answer_does_not_match_key() {
        let method = StaticKeyAuthenticationMethod::new(String::from("answer"));

        let mut authenticator = TestAuthenticator {
            challenge: Box::new(|_| {
                Ok(ChallengeResponse {
                    answers: vec![String::from("other")],
                })
            }),
            ..Default::default()
        };

        let err = method.authenticate(&mut authenticator).await.unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(err.to_string(), "Error: answer does not match key");
    }

    #[test(tokio::test)]
    async fn authenticate_should_succeed_if_answer_matches_key() {
        let method = StaticKeyAuthenticationMethod::new(String::from("answer"));

        let mut authenticator = TestAuthenticator {
            challenge: Box::new(|_| {
                Ok(ChallengeResponse {
                    answers: vec![String::from("answer")],
                })
            }),
            ..Default::default()
        };

        method.authenticate(&mut authenticator).await.unwrap();
    }
}
