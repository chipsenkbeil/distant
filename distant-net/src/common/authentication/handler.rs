use super::msg::*;
use async_trait::async_trait;
use log::*;
use std::io;

/// Interface for a handler of authentication requests.
#[async_trait]
pub trait AuthHandler {
    /// Callback when a challenge is received, returning answers to the given questions.
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse>;

    /// Callback when a verification request is received, returning true if approvided or false if
    /// unapproved.
    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse>;

    /// Callback when authentication is beginning, providing available authentication methods and
    /// returning selected authentication methods to pursue
    async fn on_initialization(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        Ok(InitializationResponse {
            methods: initialization.methods,
        })
    }

    /// Callback when authentication starts for a specific method
    #[allow(unused_variables)]
    async fn on_start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        Ok(())
    }

    /// Callback when authentication is finished and no more requests will be received
    async fn on_finished(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Callback when information is received. To fail, return an error from this function.
    #[allow(unused_variables)]
    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        Ok(())
    }

    /// Callback when an error is received. Regardless of the result returned, this will terminate
    /// the authenticator. In the situation where a custom error would be preferred, have this
    /// callback return an error.
    #[allow(unused_variables)]
    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        Ok(())
    }
}

/// Dummy implementation of [`AuthHandler`] where any challenge or verification request will
/// instantly fail.
pub struct DummyAuthHandler;

#[async_trait]
impl AuthHandler for DummyAuthHandler {
    async fn on_challenge(&mut self, _: Challenge) -> io::Result<ChallengeResponse> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    async fn on_verification(&mut self, _: Verification) -> io::Result<VerificationResponse> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Blocking implementation of [`AuthHandler`] that uses prompts to communicate challenge &
/// verification requests, receiving responses to relay back.
pub struct PromptAuthHandler<T, U> {
    text_prompt: T,
    password_prompt: U,
}

impl<T, U> PromptAuthHandler<T, U> {
    pub fn new(text_prompt: T, password_prompt: U) -> Self {
        Self {
            text_prompt,
            password_prompt,
        }
    }
}

#[async_trait]
impl<T, U> AuthHandler for PromptAuthHandler<T, U>
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
