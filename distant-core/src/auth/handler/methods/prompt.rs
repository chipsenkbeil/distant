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
