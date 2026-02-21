use std::io;

use async_trait::async_trait;

use crate::auth::handler::AuthHandler;
use crate::auth::msg::*;

/// Represents an interface for authenticating with a server.
#[async_trait]
pub trait Authenticate {
    /// Performs authentication by leveraging the `handler` for any received challenge.
    async fn authenticate(&mut self, mut handler: impl AuthHandler) -> io::Result<()>;
}

/// Represents an interface for submitting challenges for authentication.
#[async_trait]
pub trait Authenticator: Send {
    /// Issues an initialization notice and returns the response indicating which authentication
    /// methods to pursue
    async fn initialize(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse>;

    /// Issues a challenge and returns the answers to the `questions` asked.
    async fn challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse>;

    /// Requests verification of some `kind` and `text`, returning true if passed verification.
    async fn verify(&mut self, verification: Verification) -> io::Result<VerificationResponse>;

    /// Reports information with no response expected.
    async fn info(&mut self, info: Info) -> io::Result<()>;

    /// Reports an error occurred during authentication, consuming the authenticator since no more
    /// challenges should be issued.
    async fn error(&mut self, error: Error) -> io::Result<()>;

    /// Reports that the authentication has started for a specific method.
    async fn start_method(&mut self, start_method: StartMethod) -> io::Result<()>;

    /// Reports that the authentication has finished successfully, consuming the authenticator
    /// since no more challenges should be issued.
    async fn finished(&mut self) -> io::Result<()>;
}

/// Represents an implementator of [`Authenticator`] used purely for testing purposes.
#[cfg(any(test, feature = "tests"))]
pub struct TestAuthenticator {
    pub initialize: Box<dyn FnMut(Initialization) -> io::Result<InitializationResponse> + Send>,
    pub challenge: Box<dyn FnMut(Challenge) -> io::Result<ChallengeResponse> + Send>,
    pub verify: Box<dyn FnMut(Verification) -> io::Result<VerificationResponse> + Send>,
    pub info: Box<dyn FnMut(Info) -> io::Result<()> + Send>,
    pub error: Box<dyn FnMut(Error) -> io::Result<()> + Send>,
    pub start_method: Box<dyn FnMut(StartMethod) -> io::Result<()> + Send>,
    pub finished: Box<dyn FnMut() -> io::Result<()> + Send>,
}

#[cfg(any(test, feature = "tests"))]
impl Default for TestAuthenticator {
    fn default() -> Self {
        Self {
            initialize: Box::new(|x| Ok(InitializationResponse { methods: x.methods })),
            challenge: Box::new(|x| {
                Ok(ChallengeResponse {
                    answers: x.questions.into_iter().map(|x| x.text).collect(),
                })
            }),
            verify: Box::new(|_| Ok(VerificationResponse { valid: true })),
            info: Box::new(|_| Ok(())),
            error: Box::new(|_| Ok(())),
            start_method: Box::new(|_| Ok(())),
            finished: Box::new(|| Ok(())),
        }
    }
}

#[cfg(any(test, feature = "tests"))]
#[async_trait]
impl Authenticator for TestAuthenticator {
    async fn initialize(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        (self.initialize)(initialization)
    }

    async fn challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        (self.challenge)(challenge)
    }

    async fn verify(&mut self, verification: Verification) -> io::Result<VerificationResponse> {
        (self.verify)(verification)
    }

    async fn info(&mut self, info: Info) -> io::Result<()> {
        (self.info)(info)
    }

    async fn error(&mut self, error: Error) -> io::Result<()> {
        (self.error)(error)
    }

    async fn start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        (self.start_method)(start_method)
    }

    async fn finished(&mut self) -> io::Result<()> {
        (self.finished)()
    }
}
