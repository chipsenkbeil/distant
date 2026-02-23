use std::future::Future;
use std::io;
use std::pin::Pin;

use crate::auth::handler::AuthHandler;
use crate::auth::msg::*;

/// Represents an interface for authenticating with a server.
pub trait Authenticate {
    /// Performs authentication by leveraging the `handler` for any received challenge.
    fn authenticate(
        &mut self,
        handler: impl AuthHandler,
    ) -> impl Future<Output = io::Result<()>> + Send;
}

/// Represents an interface for submitting challenges for authentication.
pub trait Authenticator: Send {
    /// Issues an initialization notice and returns the response indicating which authentication
    /// methods to pursue
    fn initialize<'a>(
        &'a mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'a>>;

    /// Issues a challenge and returns the answers to the `questions` asked.
    fn challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>>;

    /// Requests verification of some `kind` and `text`, returning true if passed verification.
    fn verify<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>>;

    /// Reports information with no response expected.
    fn info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

    /// Reports an error occurred during authentication, consuming the authenticator since no more
    /// challenges should be issued.
    fn error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

    /// Reports that the authentication has started for a specific method.
    fn start_method<'a>(
        &'a mut self,
        start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

    /// Reports that the authentication has finished successfully, consuming the authenticator
    /// since no more challenges should be issued.
    fn finished<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;
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
impl Authenticator for TestAuthenticator {
    fn initialize<'a>(
        &'a mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'a>> {
        Box::pin(async move { (self.initialize)(initialization) })
    }

    fn challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async move { (self.challenge)(challenge) })
    }

    fn verify<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        Box::pin(async move { (self.verify)(verification) })
    }

    fn info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { (self.info)(info) })
    }

    fn error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { (self.error)(error) })
    }

    fn start_method<'a>(
        &'a mut self,
        start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { (self.start_method)(start_method) })
    }

    fn finished<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { (self.finished)() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_authenticator_default_initialize() {
        let mut auth = TestAuthenticator::default();
        let init = Initialization {
            methods: vec!["none".to_string()],
        };
        let resp = auth.initialize(init).await.unwrap();
        assert_eq!(resp.methods, vec!["none".to_string()]);
    }

    #[tokio::test]
    async fn test_authenticator_default_challenge() {
        let mut auth = TestAuthenticator::default();
        let challenge = Challenge {
            questions: vec![
                Question {
                    label: "q1".to_string(),
                    text: "password".to_string(),
                    options: Default::default(),
                },
                Question {
                    label: "q2".to_string(),
                    text: "token".to_string(),
                    options: Default::default(),
                },
            ],
            options: Default::default(),
        };
        let resp = auth.challenge(challenge).await.unwrap();
        assert_eq!(
            resp.answers,
            vec!["password".to_string(), "token".to_string()]
        );
    }

    #[tokio::test]
    async fn test_authenticator_default_verify() {
        let mut auth = TestAuthenticator::default();
        let verification = Verification {
            kind: VerificationKind::Host,
            text: "some host".to_string(),
        };
        let resp = auth.verify(verification).await.unwrap();
        assert!(resp.valid);
    }

    #[tokio::test]
    async fn test_authenticator_default_info() {
        let mut auth = TestAuthenticator::default();
        let info = Info {
            text: "info message".to_string(),
        };
        auth.info(info).await.unwrap();
    }

    #[tokio::test]
    async fn test_authenticator_default_error() {
        let mut auth = TestAuthenticator::default();
        let error = Error {
            kind: ErrorKind::Fatal,
            text: "error message".to_string(),
        };
        auth.error(error).await.unwrap();
    }

    #[tokio::test]
    async fn test_authenticator_default_start_method() {
        let mut auth = TestAuthenticator::default();
        let start = StartMethod {
            method: "none".to_string(),
        };
        auth.start_method(start).await.unwrap();
    }

    #[tokio::test]
    async fn test_authenticator_default_finished() {
        let mut auth = TestAuthenticator::default();
        auth.finished().await.unwrap();
    }
}
