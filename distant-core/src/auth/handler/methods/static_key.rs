use std::fmt::Display;
use std::io;

use async_trait::async_trait;
use log::*;

use crate::auth::handler::AuthMethodHandler;
use crate::auth::msg::{Challenge, ChallengeResponse, Error, Info, Verification, VerificationResponse};

/// Implementation of [`AuthMethodHandler`] that answers challenge requests using a static
/// [`HeapSecretKey`]. All other portions of method authentication are handled by another
/// [`AuthMethodHandler`].
pub struct StaticKeyAuthMethodHandler<K> {
    key: K,
    handler: Box<dyn AuthMethodHandler>,
}

impl<K> StaticKeyAuthMethodHandler<K> {
    /// Creates a new [`StaticKeyAuthMethodHandler`] that responds to challenges using a static
    /// `key`. All other requests are passed to the `handler`.
    pub fn new<T: AuthMethodHandler + 'static>(key: K, handler: T) -> Self {
        Self {
            key,
            handler: Box::new(handler),
        }
    }

    /// Creates a new [`StaticKeyAuthMethodHandler`] that responds to challenges using a static
    /// `key`. All other requests are passed automatically, meaning that verification is always
    /// approvide and info/errors are ignored.
    pub fn simple(key: K) -> Self {
        Self::new(key, {
            struct __AuthMethodHandler;

            #[async_trait]
            impl AuthMethodHandler for __AuthMethodHandler {
                #[allow(clippy::diverging_sub_expression)]
                async fn on_challenge(&mut self, _: Challenge) -> io::Result<ChallengeResponse> {
                    unreachable!("on_challenge should be handled by StaticKeyAuthMethodHandler");
                }

                async fn on_verification(
                    &mut self,
                    _: Verification,
                ) -> io::Result<VerificationResponse> {
                    Ok(VerificationResponse { valid: true })
                }

                async fn on_info(&mut self, _: Info) -> io::Result<()> {
                    Ok(())
                }

                async fn on_error(&mut self, _: Error) -> io::Result<()> {
                    Ok(())
                }
            }

            __AuthMethodHandler
        })
    }
}

#[async_trait]
impl<K> AuthMethodHandler for StaticKeyAuthMethodHandler<K>
where
    K: Display + Send,
{
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        trace!("on_challenge({challenge:?})");
        let mut answers = Vec::new();
        for question in challenge.questions.iter() {
            // Only challenges with a "key" label are allowed, all else will fail
            if question.label != "key" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Only 'key' challenges are supported",
                ));
            }
            answers.push(self.key.to_string());
        }
        Ok(ChallengeResponse { answers })
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
        trace!("on_verify({verification:?})");
        self.handler.on_verification(verification).await
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        trace!("on_info({info:?})");
        self.handler.on_info(info).await
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        trace!("on_error({error:?})");
        self.handler.on_error(error).await
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;
    use crate::auth::msg::{ErrorKind, Question, VerificationKind};

    #[test(tokio::test)]
    async fn on_challenge_should_fail_if_non_key_question_received() {
        let mut handler = StaticKeyAuthMethodHandler::simple(String::from("secret-key"));

        handler
            .on_challenge(Challenge {
                questions: vec![Question::new("test")],
                options: Default::default(),
            })
            .await
            .unwrap_err();
    }

    #[test(tokio::test)]
    async fn on_challenge_should_answer_with_stringified_key_for_key_questions() {
        let mut handler = StaticKeyAuthMethodHandler::simple(String::from("secret-key"));

        let response = handler
            .on_challenge(Challenge {
                questions: vec![Question::new("key")],
                options: Default::default(),
            })
            .await
            .unwrap();
        assert_eq!(response.answers.len(), 1, "Wrong answer set received");
        assert!(!response.answers[0].is_empty(), "Empty answer being sent");
    }

    #[test(tokio::test)]
    async fn on_verification_should_leverage_fallback_handler() {
        let mut handler = StaticKeyAuthMethodHandler::simple(String::from("secret-key"));

        let response = handler
            .on_verification(Verification {
                kind: VerificationKind::Host,
                text: "host".to_string(),
            })
            .await
            .unwrap();
        assert!(response.valid, "Unexpected result from fallback handler");
    }

    #[test(tokio::test)]
    async fn on_info_should_leverage_fallback_handler() {
        let mut handler = StaticKeyAuthMethodHandler::simple(String::from("secret-key"));

        handler
            .on_info(Info {
                text: "info".to_string(),
            })
            .await
            .unwrap();
    }

    #[test(tokio::test)]
    async fn on_error_should_leverage_fallback_handler() {
        let mut handler = StaticKeyAuthMethodHandler::simple(String::from("secret-key"));

        handler
            .on_error(Error {
                kind: ErrorKind::Error,
                text: "text".to_string(),
            })
            .await
            .unwrap();
    }
}
