use std::future::Future;
use std::io;
use std::pin::Pin;

use log::*;

use crate::auth::handler::AuthMethodHandler;
use crate::auth::msg::{
    Challenge, ChallengeResponse, Error, Info, Verification, VerificationKind, VerificationResponse,
};

/// Blocking implementation of [`AuthMethodHandler`] that uses prompts to communicate challenge &
/// verification requests, receiving responses to relay back.
pub struct PromptAuthMethodHandler<T, U> {
    text_prompt: T,
    password_prompt: U,
}

impl<T, U> PromptAuthMethodHandler<T, U> {
    pub fn new(text_prompt: T, password_prompt: U) -> Self {
        Self {
            text_prompt,
            password_prompt,
        }
    }
}

impl<T, U> AuthMethodHandler for PromptAuthMethodHandler<T, U>
where
    T: Fn(&str) -> io::Result<String> + Send + Sync + 'static,
    U: Fn(&str) -> io::Result<String> + Send + Sync + 'static,
{
    fn on_challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async move {
            trace!("on_challenge({challenge:?})");
            let mut answers = Vec::new();
            for question in challenge.questions.iter() {
                // Contains all prompt lines including same line
                let mut lines = question.text.split('\n').collect::<Vec<_>>();

                // Line that is prompt on same line as answer
                let line = lines.pop().unwrap();

                // Go ahead and display all other lines
                for line in lines.into_iter() {
                    eprintln!("{line}");
                }

                // Get an answer from user input, or use a blank string as an answer
                // if we fail to get input from the user
                let answer = (self.password_prompt)(line).unwrap_or_default();

                answers.push(answer);
            }
            Ok(ChallengeResponse { answers })
        })
    }

    fn on_verification<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        Box::pin(async move {
            trace!("on_verify({verification:?})");
            match verification.kind {
                VerificationKind::Host => {
                    eprintln!("{}", verification.text);

                    let answer = (self.text_prompt)("Enter [y/N]> ")?;
                    trace!("Verify? Answer = '{answer}'");
                    Ok(VerificationResponse {
                        valid: matches!(answer.trim(), "y" | "Y" | "yes" | "YES"),
                    })
                }
                x => {
                    error!("Unsupported verify kind: {x}");
                    Ok(VerificationResponse { valid: false })
                }
            }
        })
    }

    fn on_info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            trace!("on_info({info:?})");
            println!("{}", info.text);
            Ok(())
        })
    }

    fn on_error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            trace!("on_error({error:?})");
            eprintln!("{}: {}", error.kind, error.text);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::msg::*;

    use std::collections::HashMap;

    /// Helper to create a Challenge with the given question texts.
    fn make_challenge(questions: Vec<&str>) -> Challenge {
        Challenge {
            questions: questions
                .into_iter()
                .map(|text| Question {
                    label: text.to_string(),
                    text: text.to_string(),
                    options: HashMap::new(),
                })
                .collect(),
            options: HashMap::new(),
        }
    }

    #[test_log::test(tokio::test)]
    async fn on_challenge_single_question_returns_password_prompt_result() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("ignored".to_string()),
            |_: &str| Ok("my_password".to_string()),
        );

        let challenge = make_challenge(vec!["Password: "]);
        let response = handler.on_challenge(challenge).await.unwrap();

        assert_eq!(response.answers, vec!["my_password".to_string()]);
    }

    #[test_log::test(tokio::test)]
    async fn on_challenge_multiple_questions_returns_all_answers() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("ignored".to_string()),
            |prompt: &str| Ok(format!("answer_for_{prompt}")),
        );

        let challenge = make_challenge(vec!["Q1: ", "Q2: ", "Q3: "]);
        let response = handler.on_challenge(challenge).await.unwrap();

        assert_eq!(response.answers.len(), 3);
        assert_eq!(response.answers[0], "answer_for_Q1: ");
        assert_eq!(response.answers[1], "answer_for_Q2: ");
        assert_eq!(response.answers[2], "answer_for_Q3: ");
    }

    #[test_log::test(tokio::test)]
    async fn on_challenge_uses_empty_string_when_prompt_fails() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("ignored".to_string()),
            |_: &str| Err(io::Error::new(io::ErrorKind::Other, "prompt failed")),
        );

        let challenge = make_challenge(vec!["Password: "]);
        let response = handler.on_challenge(challenge).await.unwrap();

        assert_eq!(response.answers, vec!["".to_string()]);
    }

    #[test_log::test(tokio::test)]
    async fn on_challenge_multiline_question_text() {
        // When text contains newlines, on_challenge splits by '\n',
        // prints all lines except the last via eprintln, and prompts
        // the password_prompt with the last line.
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("ignored".to_string()),
            |prompt: &str| Ok(format!("answer_for_{prompt}")),
        );

        let challenge = Challenge {
            questions: vec![Question {
                label: "multi".to_string(),
                text: "Line 1\nLine 2\nPrompt: ".to_string(),
                options: HashMap::new(),
            }],
            options: HashMap::new(),
        };

        let response = handler.on_challenge(challenge).await.unwrap();

        // The password_prompt should receive only the last line "Prompt: "
        assert_eq!(response.answers, vec!["answer_for_Prompt: ".to_string()]);
    }

    #[test_log::test(tokio::test)]
    async fn on_verification_host_kind_y_returns_valid_true() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("y".to_string()),
            |_: &str| Ok("ignored".to_string()),
        );

        let verification = Verification {
            kind: VerificationKind::Host,
            text: "Trust this host?".to_string(),
        };

        let response = handler.on_verification(verification).await.unwrap();
        assert!(response.valid);
    }

    #[test_log::test(tokio::test)]
    async fn on_verification_host_kind_uppercase_y_returns_valid_true() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("Y".to_string()),
            |_: &str| Ok("ignored".to_string()),
        );

        let verification = Verification {
            kind: VerificationKind::Host,
            text: "Trust this host?".to_string(),
        };

        let response = handler.on_verification(verification).await.unwrap();
        assert!(response.valid);
    }

    #[test_log::test(tokio::test)]
    async fn on_verification_host_kind_yes_returns_valid_true() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("yes".to_string()),
            |_: &str| Ok("ignored".to_string()),
        );

        let verification = Verification {
            kind: VerificationKind::Host,
            text: "Trust this host?".to_string(),
        };

        let response = handler.on_verification(verification).await.unwrap();
        assert!(response.valid);
    }

    #[test_log::test(tokio::test)]
    async fn on_verification_host_kind_n_returns_valid_false() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("n".to_string()),
            |_: &str| Ok("ignored".to_string()),
        );

        let verification = Verification {
            kind: VerificationKind::Host,
            text: "Trust this host?".to_string(),
        };

        let response = handler.on_verification(verification).await.unwrap();
        assert!(!response.valid);
    }

    #[test_log::test(tokio::test)]
    async fn on_verification_host_kind_empty_returns_valid_false() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("".to_string()),
            |_: &str| Ok("ignored".to_string()),
        );

        let verification = Verification {
            kind: VerificationKind::Host,
            text: "Trust this host?".to_string(),
        };

        let response = handler.on_verification(verification).await.unwrap();
        assert!(!response.valid);
    }

    #[test_log::test(tokio::test)]
    async fn on_verification_unknown_kind_returns_valid_false() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("y".to_string()),
            |_: &str| Ok("ignored".to_string()),
        );

        let verification = Verification {
            kind: VerificationKind::Unknown,
            text: "Some unknown verification".to_string(),
        };

        let response = handler.on_verification(verification).await.unwrap();
        assert!(!response.valid);
    }

    #[test_log::test(tokio::test)]
    async fn on_verification_host_kind_when_text_prompt_fails_returns_error() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Err(io::Error::new(io::ErrorKind::Other, "prompt failed")),
            |_: &str| Ok("ignored".to_string()),
        );

        let verification = Verification {
            kind: VerificationKind::Host,
            text: "Trust this host?".to_string(),
        };

        let err = handler.on_verification(verification).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test_log::test(tokio::test)]
    async fn on_info_returns_ok() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("ignored".to_string()),
            |_: &str| Ok("ignored".to_string()),
        );

        let info = Info {
            text: "Some informational message".to_string(),
        };

        let result = handler.on_info(info).await;
        assert!(result.is_ok());
    }

    #[test_log::test(tokio::test)]
    async fn on_error_returns_ok() {
        let mut handler = PromptAuthMethodHandler::new(
            |_: &str| Ok("ignored".to_string()),
            |_: &str| Ok("ignored".to_string()),
        );

        let error = Error {
            kind: ErrorKind::Fatal,
            text: "Something went wrong".to_string(),
        };

        let result = handler.on_error(error).await;
        assert!(result.is_ok());
    }
}
