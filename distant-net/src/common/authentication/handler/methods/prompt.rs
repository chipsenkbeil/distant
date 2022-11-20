use super::{
    AuthMethodHandler, Challenge, ChallengeResponse, Error, Info, Verification, VerificationKind,
    VerificationResponse,
};
use async_trait::async_trait;
use log::*;
use std::io;

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

#[async_trait]
impl<T, U> AuthMethodHandler for PromptAuthMethodHandler<T, U>
where
    T: Fn(&str) -> io::Result<String> + Send + Sync + 'static,
    U: Fn(&str) -> io::Result<String> + Send + Sync + 'static,
{
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        trace!("on_challenge({challenge:?})");
        let mut answers = Vec::new();
        for question in challenge.questions.iter() {
            // Contains all prompt lines including same line
            let mut lines = question.text.split('\n').collect::<Vec<_>>();

            // Line that is prompt on same line as answer
            let line = lines.pop().unwrap();

            // Go ahead and display all other lines
            for line in lines.into_iter() {
                eprintln!("{}", line);
            }

            // Get an answer from user input, or use a blank string as an answer
            // if we fail to get input from the user
            let answer = (self.password_prompt)(line).unwrap_or_default();

            answers.push(answer);
        }
        Ok(ChallengeResponse { answers })
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
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
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        trace!("on_info({info:?})");
        println!("{}", info.text);
        Ok(())
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        trace!("on_error({error:?})");
        eprintln!("{}: {}", error.kind, error.text);
        Ok(())
    }
}
