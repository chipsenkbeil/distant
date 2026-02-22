use std::collections::HashMap;
use std::fmt::Display;
use std::future::Future;
use std::io;
use std::pin::Pin;

use crate::auth::authenticator::Authenticator;
use crate::auth::msg::*;

mod methods;
pub use methods::*;

/// Interface for a handler of authentication requests for all methods.
pub trait AuthHandler: AuthMethodHandler + Send {
    /// Callback when authentication is beginning, providing available authentication methods and
    /// returning selected authentication methods to pursue.
    fn on_initialization<'a>(
        &'a mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'a>> {
        Box::pin(async move {
            Ok(InitializationResponse {
                methods: initialization.methods,
            })
        })
    }

    /// Callback when authentication starts for a specific method.
    #[allow(unused_variables)]
    fn on_start_method<'a>(
        &'a mut self,
        start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { Ok(()) })
    }

    /// Callback when authentication is finished and no more requests will be received.
    fn on_finished<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { Ok(()) })
    }
}

/// Dummy implementation of [`AuthHandler`] where any challenge or verification request will
/// instantly fail.
pub struct DummyAuthHandler;

impl AuthHandler for DummyAuthHandler {}

impl AuthMethodHandler for DummyAuthHandler {
    fn on_challenge<'a>(
        &'a mut self,
        _: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async move { Err(io::Error::from(io::ErrorKind::Unsupported)) })
    }

    fn on_verification<'a>(
        &'a mut self,
        _: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        Box::pin(async move { Err(io::Error::from(io::ErrorKind::Unsupported)) })
    }

    fn on_info<'a>(
        &'a mut self,
        _: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { Err(io::Error::from(io::ErrorKind::Unsupported)) })
    }

    fn on_error<'a>(
        &'a mut self,
        _: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { Err(io::Error::from(io::ErrorKind::Unsupported)) })
    }
}

/// Implementation of [`AuthHandler`] that uses the same [`AuthMethodHandler`] for all methods.
pub struct SingleAuthHandler(Box<dyn AuthMethodHandler>);

impl SingleAuthHandler {
    pub fn new<T: AuthMethodHandler + 'static>(method_handler: T) -> Self {
        Self(Box::new(method_handler))
    }
}

impl AuthHandler for SingleAuthHandler {}

impl AuthMethodHandler for SingleAuthHandler {
    fn on_challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async move { self.0.on_challenge(challenge).await })
    }

    fn on_verification<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        Box::pin(async move { self.0.on_verification(verification).await })
    }

    fn on_info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { self.0.on_info(info).await })
    }

    fn on_error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { self.0.on_error(error).await })
    }
}

/// Implementation of [`AuthHandler`] that maintains a map of [`AuthMethodHandler`] implementations
/// for specific methods, invoking [`on_challenge`], [`on_verification`], [`on_info`], and
/// [`on_error`] for a specific handler based on an associated id.
///
/// [`on_challenge`]: AuthMethodHandler::on_challenge
/// [`on_verification`]: AuthMethodHandler::on_verification
/// [`on_info`]: AuthMethodHandler::on_info
/// [`on_error`]: AuthMethodHandler::on_error
pub struct AuthHandlerMap {
    active: String,
    map: HashMap<&'static str, Box<dyn AuthMethodHandler>>,
}

impl AuthHandlerMap {
    /// Creates a new, empty map of auth method handlers.
    pub fn new() -> Self {
        Self {
            active: String::new(),
            map: HashMap::new(),
        }
    }

    /// Returns the `id` of the active [`AuthMethodHandler`].
    pub fn active_id(&self) -> &str {
        &self.active
    }

    /// Sets the active [`AuthMethodHandler`] by its `id`.
    pub fn set_active_id(&mut self, id: impl Into<String>) {
        self.active = id.into();
    }

    /// Inserts the specified `handler` into the map, associating it with `id` for determining the
    /// method that would trigger this handler.
    pub fn insert_method_handler<T: AuthMethodHandler + 'static>(
        &mut self,
        id: &'static str,
        handler: T,
    ) -> Option<Box<dyn AuthMethodHandler>> {
        self.map.insert(id, Box::new(handler))
    }

    /// Removes a handler with the associated `id`.
    pub fn remove_method_handler(
        &mut self,
        id: &'static str,
    ) -> Option<Box<dyn AuthMethodHandler>> {
        self.map.remove(id)
    }

    /// Retrieves a mutable reference to the active [`AuthMethodHandler`] with the specified `id`,
    /// returning an error if no handler for the active id is found.
    pub fn get_mut_active_method_handler_or_error(
        &mut self,
    ) -> io::Result<&mut (dyn AuthMethodHandler + 'static)> {
        let id = self.active.clone();
        self.get_mut_active_method_handler()
            .ok_or_else(|| io::Error::other(format!("No active handler for {id}")))
    }

    /// Retrieves a mutable reference to the active [`AuthMethodHandler`] with the specified `id`.
    pub fn get_mut_active_method_handler(
        &mut self,
    ) -> Option<&mut (dyn AuthMethodHandler + 'static)> {
        // TODO: Optimize this
        self.get_mut_method_handler(&self.active.clone())
    }

    /// Retrieves a mutable reference to the [`AuthMethodHandler`] with the specified `id`.
    pub fn get_mut_method_handler(
        &mut self,
        id: &str,
    ) -> Option<&mut (dyn AuthMethodHandler + 'static)> {
        self.map.get_mut(id).map(|h| h.as_mut())
    }
}

impl AuthHandlerMap {
    /// Consumes the map, returning a new map that supports the `static_key` method.
    pub fn with_static_key<K>(mut self, key: K) -> Self
    where
        K: Display + Send + 'static,
    {
        self.insert_method_handler("static_key", StaticKeyAuthMethodHandler::simple(key));
        self
    }
}

impl Default for AuthHandlerMap {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthHandler for AuthHandlerMap {
    fn on_initialization<'a>(
        &'a mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'a>> {
        Box::pin(async move {
            let methods = initialization
                .methods
                .into_iter()
                .filter(|method| self.map.contains_key(method.as_str()))
                .collect();

            Ok(InitializationResponse { methods })
        })
    }

    fn on_start_method<'a>(
        &'a mut self,
        start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.set_active_id(start_method.method);
            Ok(())
        })
    }

    fn on_finished<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { Ok(()) })
    }
}

impl AuthMethodHandler for AuthHandlerMap {
    fn on_challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async move {
            let handler = self.get_mut_active_method_handler_or_error()?;
            handler.on_challenge(challenge).await
        })
    }

    fn on_verification<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        Box::pin(async move {
            let handler = self.get_mut_active_method_handler_or_error()?;
            handler.on_verification(verification).await
        })
    }

    fn on_info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let handler = self.get_mut_active_method_handler_or_error()?;
            handler.on_info(info).await
        })
    }

    fn on_error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let handler = self.get_mut_active_method_handler_or_error()?;
            handler.on_error(error).await
        })
    }
}

/// Implementation of [`AuthHandler`] that redirects all requests to an [`Authenticator`].
pub struct ProxyAuthHandler<'a>(&'a mut dyn Authenticator);

impl<'a> ProxyAuthHandler<'a> {
    pub fn new(authenticator: &'a mut dyn Authenticator) -> Self {
        Self(authenticator)
    }
}

impl<'a> AuthHandler for ProxyAuthHandler<'a> {
    fn on_initialization<'b>(
        &'b mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'b>> {
        Box::pin(async move { Authenticator::initialize(self.0, initialization).await })
    }

    fn on_start_method<'b>(
        &'b mut self,
        start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move { Authenticator::start_method(self.0, start_method).await })
    }

    fn on_finished<'b>(&'b mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move { Authenticator::finished(self.0).await })
    }
}

impl<'a> AuthMethodHandler for ProxyAuthHandler<'a> {
    fn on_challenge<'b>(
        &'b mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'b>> {
        Box::pin(async move { Authenticator::challenge(self.0, challenge).await })
    }

    fn on_verification<'b>(
        &'b mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'b>> {
        Box::pin(async move { Authenticator::verify(self.0, verification).await })
    }

    fn on_info<'b>(
        &'b mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move { Authenticator::info(self.0, info).await })
    }

    fn on_error<'b>(
        &'b mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move { Authenticator::error(self.0, error).await })
    }
}

/// Implementation of [`AuthHandler`] that holds a mutable reference to another [`AuthHandler`]
/// trait object to use underneath.
pub struct DynAuthHandler<'a>(&'a mut dyn AuthHandler);

impl<'a> DynAuthHandler<'a> {
    pub fn new(handler: &'a mut dyn AuthHandler) -> Self {
        Self(handler)
    }
}

impl<'a, T: AuthHandler> From<&'a mut T> for DynAuthHandler<'a> {
    fn from(handler: &'a mut T) -> Self {
        Self::new(handler as &mut dyn AuthHandler)
    }
}

impl<'a> AuthHandler for DynAuthHandler<'a> {
    fn on_initialization<'b>(
        &'b mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'b>> {
        Box::pin(async move { self.0.on_initialization(initialization).await })
    }

    fn on_start_method<'b>(
        &'b mut self,
        start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move { self.0.on_start_method(start_method).await })
    }

    fn on_finished<'b>(&'b mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move { self.0.on_finished().await })
    }
}

impl<'a> AuthMethodHandler for DynAuthHandler<'a> {
    fn on_challenge<'b>(
        &'b mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'b>> {
        Box::pin(async move { self.0.on_challenge(challenge).await })
    }

    fn on_verification<'b>(
        &'b mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'b>> {
        Box::pin(async move { self.0.on_verification(verification).await })
    }

    fn on_info<'b>(
        &'b mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move { self.0.on_info(info).await })
    }

    fn on_error<'b>(
        &'b mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move { self.0.on_error(error).await })
    }
}

/// Represents an implementator of [`AuthHandler`] used purely for testing purposes.
#[cfg(any(test, feature = "tests"))]
pub struct TestAuthHandler {
    pub on_initialization:
        Box<dyn FnMut(Initialization) -> io::Result<InitializationResponse> + Send>,
    pub on_challenge: Box<dyn FnMut(Challenge) -> io::Result<ChallengeResponse> + Send>,
    pub on_verification: Box<dyn FnMut(Verification) -> io::Result<VerificationResponse> + Send>,
    pub on_info: Box<dyn FnMut(Info) -> io::Result<()> + Send>,
    pub on_error: Box<dyn FnMut(Error) -> io::Result<()> + Send>,
    pub on_start_method: Box<dyn FnMut(StartMethod) -> io::Result<()> + Send>,
    pub on_finished: Box<dyn FnMut() -> io::Result<()> + Send>,
}

#[cfg(any(test, feature = "tests"))]
impl Default for TestAuthHandler {
    fn default() -> Self {
        Self {
            on_initialization: Box::new(|x| Ok(InitializationResponse { methods: x.methods })),
            on_challenge: Box::new(|x| {
                Ok(ChallengeResponse {
                    answers: x.questions.into_iter().map(|x| x.text).collect(),
                })
            }),
            on_verification: Box::new(|_| Ok(VerificationResponse { valid: true })),
            on_info: Box::new(|_| Ok(())),
            on_error: Box::new(|_| Ok(())),
            on_start_method: Box::new(|_| Ok(())),
            on_finished: Box::new(|| Ok(())),
        }
    }
}

#[cfg(any(test, feature = "tests"))]
impl AuthHandler for TestAuthHandler {
    fn on_initialization<'a>(
        &'a mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'a>> {
        Box::pin(async move { (self.on_initialization)(initialization) })
    }

    fn on_start_method<'a>(
        &'a mut self,
        start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { (self.on_start_method)(start_method) })
    }

    fn on_finished<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { (self.on_finished)() })
    }
}

#[cfg(any(test, feature = "tests"))]
impl AuthMethodHandler for TestAuthHandler {
    fn on_challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async move { (self.on_challenge)(challenge) })
    }

    fn on_verification<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        Box::pin(async move { (self.on_verification)(verification) })
    }

    fn on_info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { (self.on_info)(info) })
    }

    fn on_error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { (self.on_error)(error) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::authenticator::TestAuthenticator;
    use test_log::test;

    // ---------------------------------------------------------------------------
    // Helper: a simple AuthMethodHandler for use in tests that records what it
    // receives and returns canned responses.
    // ---------------------------------------------------------------------------
    struct RecordingMethodHandler {
        challenge_answers: Vec<String>,
        verification_valid: bool,
    }

    impl RecordingMethodHandler {
        fn new(answers: Vec<String>, valid: bool) -> Self {
            Self {
                challenge_answers: answers,
                verification_valid: valid,
            }
        }
    }

    impl AuthMethodHandler for RecordingMethodHandler {
        fn on_challenge<'a>(
            &'a mut self,
            _challenge: Challenge,
        ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
            Box::pin(async move {
                Ok(ChallengeResponse {
                    answers: self.challenge_answers.clone(),
                })
            })
        }

        fn on_verification<'a>(
            &'a mut self,
            _verification: Verification,
        ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
            Box::pin(async move {
                Ok(VerificationResponse {
                    valid: self.verification_valid,
                })
            })
        }

        fn on_info<'a>(
            &'a mut self,
            _info: Info,
        ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
            Box::pin(async move { Ok(()) })
        }

        fn on_error<'a>(
            &'a mut self,
            _error: Error,
        ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
            Box::pin(async move { Ok(()) })
        }
    }

    /// A method handler that always fails, useful for testing error propagation.
    struct FailingMethodHandler;

    impl AuthMethodHandler for FailingMethodHandler {
        fn on_challenge<'a>(
            &'a mut self,
            _: Challenge,
        ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
            Box::pin(async move { Err(io::Error::other("challenge failed")) })
        }

        fn on_verification<'a>(
            &'a mut self,
            _: Verification,
        ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
            Box::pin(async move { Err(io::Error::other("verification failed")) })
        }

        fn on_info<'a>(
            &'a mut self,
            _: Info,
        ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
            Box::pin(async move { Err(io::Error::other("info failed")) })
        }

        fn on_error<'a>(
            &'a mut self,
            _: Error,
        ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
            Box::pin(async move { Err(io::Error::other("error failed")) })
        }
    }

    // Helper to create a simple challenge
    fn make_challenge() -> Challenge {
        Challenge {
            questions: vec![Question::new("test-question")],
            options: HashMap::new(),
        }
    }

    // Helper to create a simple verification
    fn make_verification() -> Verification {
        Verification {
            kind: VerificationKind::Host,
            text: "verify-text".to_string(),
        }
    }

    // Helper to create a simple info
    fn make_info() -> Info {
        Info {
            text: "info-text".to_string(),
        }
    }

    // Helper to create a simple error
    fn make_error() -> Error {
        Error {
            kind: ErrorKind::Error,
            text: "error-text".to_string(),
        }
    }

    // =======================================================================
    // DummyAuthHandler tests
    // =======================================================================

    #[test(tokio::test)]
    async fn dummy_auth_handler_on_challenge_returns_unsupported() {
        let mut handler = DummyAuthHandler;
        let err = handler.on_challenge(make_challenge()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test(tokio::test)]
    async fn dummy_auth_handler_on_verification_returns_unsupported() {
        let mut handler = DummyAuthHandler;
        let err = handler
            .on_verification(make_verification())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test(tokio::test)]
    async fn dummy_auth_handler_on_info_returns_unsupported() {
        let mut handler = DummyAuthHandler;
        let err = handler.on_info(make_info()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test(tokio::test)]
    async fn dummy_auth_handler_on_error_returns_unsupported() {
        let mut handler = DummyAuthHandler;
        let err = handler.on_error(make_error()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }

    #[test(tokio::test)]
    async fn dummy_auth_handler_on_initialization_returns_all_methods() {
        let mut handler = DummyAuthHandler;
        let init = Initialization {
            methods: vec!["method_a".to_string(), "method_b".to_string()],
        };
        let response = handler.on_initialization(init.clone()).await.unwrap();
        assert_eq!(response.methods, init.methods);
    }

    #[test(tokio::test)]
    async fn dummy_auth_handler_on_start_method_succeeds() {
        let mut handler = DummyAuthHandler;
        let start = StartMethod {
            method: "some_method".to_string(),
        };
        handler.on_start_method(start).await.unwrap();
    }

    #[test(tokio::test)]
    async fn dummy_auth_handler_on_finished_succeeds() {
        let mut handler = DummyAuthHandler;
        handler.on_finished().await.unwrap();
    }

    // =======================================================================
    // SingleAuthHandler tests
    // =======================================================================

    #[test(tokio::test)]
    async fn single_auth_handler_delegates_on_challenge() {
        let inner = RecordingMethodHandler::new(vec!["answer1".to_string()], true);
        let mut handler = SingleAuthHandler::new(inner);
        let response = handler.on_challenge(make_challenge()).await.unwrap();
        assert_eq!(response.answers, vec!["answer1".to_string()]);
    }

    #[test(tokio::test)]
    async fn single_auth_handler_delegates_on_verification() {
        let inner = RecordingMethodHandler::new(vec![], false);
        let mut handler = SingleAuthHandler::new(inner);
        let response = handler.on_verification(make_verification()).await.unwrap();
        assert!(!response.valid);
    }

    #[test(tokio::test)]
    async fn single_auth_handler_delegates_on_info() {
        let inner = RecordingMethodHandler::new(vec![], true);
        let mut handler = SingleAuthHandler::new(inner);
        handler.on_info(make_info()).await.unwrap();
    }

    #[test(tokio::test)]
    async fn single_auth_handler_delegates_on_error() {
        let inner = RecordingMethodHandler::new(vec![], true);
        let mut handler = SingleAuthHandler::new(inner);
        handler.on_error(make_error()).await.unwrap();
    }

    #[test(tokio::test)]
    async fn single_auth_handler_propagates_inner_failure() {
        let mut handler = SingleAuthHandler::new(FailingMethodHandler);
        let err = handler.on_challenge(make_challenge()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "challenge failed");
    }

    #[test(tokio::test)]
    async fn single_auth_handler_on_initialization_returns_all_methods() {
        let inner = RecordingMethodHandler::new(vec![], true);
        let mut handler = SingleAuthHandler::new(inner);
        let init = Initialization {
            methods: vec!["x".to_string(), "y".to_string()],
        };
        let response = handler.on_initialization(init.clone()).await.unwrap();
        assert_eq!(response.methods, init.methods);
    }

    #[test(tokio::test)]
    async fn single_auth_handler_on_start_method_succeeds() {
        let inner = RecordingMethodHandler::new(vec![], true);
        let mut handler = SingleAuthHandler::new(inner);
        handler
            .on_start_method(StartMethod {
                method: "m".to_string(),
            })
            .await
            .unwrap();
    }

    #[test(tokio::test)]
    async fn single_auth_handler_on_finished_succeeds() {
        let inner = RecordingMethodHandler::new(vec![], true);
        let mut handler = SingleAuthHandler::new(inner);
        handler.on_finished().await.unwrap();
    }

    // =======================================================================
    // AuthHandlerMap tests
    // =======================================================================

    #[test]
    fn auth_handler_map_new_creates_empty_map_with_empty_active_id() {
        let map = AuthHandlerMap::new();
        assert_eq!(map.active_id(), "");
    }

    #[test]
    fn auth_handler_map_default_creates_empty_map() {
        let map = AuthHandlerMap::default();
        assert_eq!(map.active_id(), "");
    }

    #[test]
    fn auth_handler_map_set_active_id_and_active_id() {
        let mut map = AuthHandlerMap::new();
        map.set_active_id("my_method");
        assert_eq!(map.active_id(), "my_method");
    }

    #[test]
    fn auth_handler_map_insert_method_handler_returns_none_for_new_id() {
        let mut map = AuthHandlerMap::new();
        let prev = map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], true));
        assert!(prev.is_none());
    }

    #[test]
    fn auth_handler_map_insert_method_handler_returns_previous_for_existing_id() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], true));
        let prev = map.insert_method_handler(
            "method_a",
            RecordingMethodHandler::new(vec!["replaced".to_string()], false),
        );
        assert!(prev.is_some());
    }

    #[test]
    fn auth_handler_map_remove_method_handler_returns_some_for_existing() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], true));
        let removed = map.remove_method_handler("method_a");
        assert!(removed.is_some());
    }

    #[test]
    fn auth_handler_map_remove_method_handler_returns_none_for_missing() {
        let mut map = AuthHandlerMap::new();
        let removed = map.remove_method_handler("nonexistent");
        assert!(removed.is_none());
    }

    #[test]
    fn auth_handler_map_get_mut_method_handler_returns_some_for_existing() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], true));
        assert!(map.get_mut_method_handler("method_a").is_some());
    }

    #[test]
    fn auth_handler_map_get_mut_method_handler_returns_none_for_missing() {
        let mut map = AuthHandlerMap::new();
        assert!(map.get_mut_method_handler("nonexistent").is_none());
    }

    #[test]
    fn auth_handler_map_get_mut_active_method_handler_returns_none_when_no_active() {
        let mut map = AuthHandlerMap::new();
        // Active id is "" by default, no handler registered for ""
        assert!(map.get_mut_active_method_handler().is_none());
    }

    #[test]
    fn auth_handler_map_get_mut_active_method_handler_returns_none_when_active_not_in_map() {
        let mut map = AuthHandlerMap::new();
        map.set_active_id("missing_method");
        assert!(map.get_mut_active_method_handler().is_none());
    }

    #[test]
    fn auth_handler_map_get_mut_active_method_handler_returns_some_when_active_in_map() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], true));
        map.set_active_id("method_a");
        assert!(map.get_mut_active_method_handler().is_some());
    }

    #[test]
    fn auth_handler_map_get_mut_active_method_handler_or_error_returns_err_when_no_active() {
        let mut map = AuthHandlerMap::new();
        match map.get_mut_active_method_handler_or_error() {
            Err(err) => {
                assert_eq!(err.kind(), io::ErrorKind::Other);
                assert!(err.to_string().contains("No active handler"));
            }
            Ok(_) => panic!("Expected error but got Ok"),
        }
    }

    #[test]
    fn auth_handler_map_get_mut_active_method_handler_or_error_returns_err_when_active_not_in_map()
    {
        let mut map = AuthHandlerMap::new();
        map.set_active_id("missing");
        match map.get_mut_active_method_handler_or_error() {
            Err(err) => {
                assert_eq!(err.kind(), io::ErrorKind::Other);
                assert!(err.to_string().contains("No active handler for missing"));
            }
            Ok(_) => panic!("Expected error but got Ok"),
        }
    }

    #[test]
    fn auth_handler_map_get_mut_active_method_handler_or_error_returns_ok_when_active_found() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], true));
        map.set_active_id("method_a");
        assert!(map.get_mut_active_method_handler_or_error().is_ok());
    }

    #[test(tokio::test)]
    async fn auth_handler_map_on_initialization_filters_to_known_methods() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("known_a", RecordingMethodHandler::new(vec![], true));
        map.insert_method_handler("known_b", RecordingMethodHandler::new(vec![], true));

        let init = Initialization {
            methods: vec![
                "known_a".to_string(),
                "unknown".to_string(),
                "known_b".to_string(),
            ],
        };
        let response = map.on_initialization(init).await.unwrap();
        assert_eq!(response.methods.len(), 2);
        assert!(response.methods.contains(&"known_a".to_string()));
        assert!(response.methods.contains(&"known_b".to_string()));
        assert!(!response.methods.contains(&"unknown".to_string()));
    }

    #[test(tokio::test)]
    async fn auth_handler_map_on_initialization_returns_empty_when_no_methods_match() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("my_method", RecordingMethodHandler::new(vec![], true));

        let init = Initialization {
            methods: vec!["other_method".to_string()],
        };
        let response = map.on_initialization(init).await.unwrap();
        assert!(response.methods.is_empty());
    }

    #[test(tokio::test)]
    async fn auth_handler_map_on_start_method_sets_active_id() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], true));

        let start = StartMethod {
            method: "method_a".to_string(),
        };
        map.on_start_method(start).await.unwrap();
        assert_eq!(map.active_id(), "method_a");
    }

    #[test(tokio::test)]
    async fn auth_handler_map_on_finished_succeeds() {
        let mut map = AuthHandlerMap::new();
        map.on_finished().await.unwrap();
    }

    #[test(tokio::test)]
    async fn auth_handler_map_challenge_delegates_to_active_handler() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler(
            "method_a",
            RecordingMethodHandler::new(vec!["ans".to_string()], true),
        );
        map.set_active_id("method_a");

        let response = map.on_challenge(make_challenge()).await.unwrap();
        assert_eq!(response.answers, vec!["ans".to_string()]);
    }

    #[test(tokio::test)]
    async fn auth_handler_map_verification_delegates_to_active_handler() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], false));
        map.set_active_id("method_a");

        let response = map.on_verification(make_verification()).await.unwrap();
        assert!(!response.valid);
    }

    #[test(tokio::test)]
    async fn auth_handler_map_info_delegates_to_active_handler() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], true));
        map.set_active_id("method_a");

        map.on_info(make_info()).await.unwrap();
    }

    #[test(tokio::test)]
    async fn auth_handler_map_error_delegates_to_active_handler() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("method_a", RecordingMethodHandler::new(vec![], true));
        map.set_active_id("method_a");

        map.on_error(make_error()).await.unwrap();
    }

    #[test(tokio::test)]
    async fn auth_handler_map_challenge_fails_when_no_active_handler() {
        let mut map = AuthHandlerMap::new();
        let err = map.on_challenge(make_challenge()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test(tokio::test)]
    async fn auth_handler_map_verification_fails_when_no_active_handler() {
        let mut map = AuthHandlerMap::new();
        let err = map.on_verification(make_verification()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test(tokio::test)]
    async fn auth_handler_map_info_fails_when_no_active_handler() {
        let mut map = AuthHandlerMap::new();
        let err = map.on_info(make_info()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test(tokio::test)]
    async fn auth_handler_map_error_fails_when_no_active_handler() {
        let mut map = AuthHandlerMap::new();
        let err = map.on_error(make_error()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test(tokio::test)]
    async fn auth_handler_map_challenge_fails_when_active_id_not_in_map() {
        let mut map = AuthHandlerMap::new();
        map.set_active_id("nonexistent");
        let err = map.on_challenge(make_challenge()).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(
            err.to_string()
                .contains("No active handler for nonexistent"),
            "Unexpected error message: {}",
            err
        );
    }

    #[test(tokio::test)]
    async fn auth_handler_map_with_static_key_inserts_static_key_handler() {
        let mut map = AuthHandlerMap::new().with_static_key("my-secret");
        map.set_active_id("static_key");

        // The static key handler responds to "key" label challenges
        let challenge = Challenge {
            questions: vec![Question::new("key")],
            options: HashMap::new(),
        };
        let response = map.on_challenge(challenge).await.unwrap();
        assert_eq!(response.answers, vec!["my-secret".to_string()]);
    }

    #[test(tokio::test)]
    async fn auth_handler_map_with_static_key_appears_in_initialization_filter() {
        let map = AuthHandlerMap::new().with_static_key("secret");
        let init = Initialization {
            methods: vec!["static_key".to_string(), "other".to_string()],
        };
        // We need a mutable reference for on_initialization
        let mut map = map;
        let response = map.on_initialization(init).await.unwrap();
        assert_eq!(response.methods, vec!["static_key".to_string()]);
    }

    #[test(tokio::test)]
    async fn auth_handler_map_active_handler_propagates_inner_errors() {
        let mut map = AuthHandlerMap::new();
        map.insert_method_handler("fail_method", FailingMethodHandler);
        map.set_active_id("fail_method");

        let err = map.on_challenge(make_challenge()).await.unwrap_err();
        assert_eq!(err.to_string(), "challenge failed");

        let err = map.on_verification(make_verification()).await.unwrap_err();
        assert_eq!(err.to_string(), "verification failed");

        let err = map.on_info(make_info()).await.unwrap_err();
        assert_eq!(err.to_string(), "info failed");

        let err = map.on_error(make_error()).await.unwrap_err();
        assert_eq!(err.to_string(), "error failed");
    }

    // =======================================================================
    // ProxyAuthHandler tests
    // =======================================================================

    #[test(tokio::test)]
    async fn proxy_auth_handler_delegates_on_initialization() {
        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_init| {
                Ok(InitializationResponse {
                    methods: vec!["proxied".to_string()],
                })
            }),
            ..Default::default()
        };

        let mut handler = ProxyAuthHandler::new(&mut authenticator);
        let init = Initialization {
            methods: vec!["a".to_string(), "b".to_string()],
        };
        let response = handler.on_initialization(init).await.unwrap();
        assert_eq!(response.methods, vec!["proxied".to_string()]);
    }

    #[test(tokio::test)]
    async fn proxy_auth_handler_delegates_on_start_method() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        let mut authenticator = TestAuthenticator {
            start_method: Box::new(move |sm| {
                tx.send(sm.method.clone()).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let mut handler = ProxyAuthHandler::new(&mut authenticator);
        handler
            .on_start_method(StartMethod {
                method: "my_method".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(rx.recv().unwrap(), "my_method");
    }

    #[test(tokio::test)]
    async fn proxy_auth_handler_delegates_on_finished() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        let mut authenticator = TestAuthenticator {
            finished: Box::new(move || {
                tx.send(()).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let mut handler = ProxyAuthHandler::new(&mut authenticator);
        handler.on_finished().await.unwrap();
        rx.recv().unwrap();
    }

    #[test(tokio::test)]
    async fn proxy_auth_handler_delegates_on_challenge() {
        let mut authenticator = TestAuthenticator {
            challenge: Box::new(|_c| {
                Ok(ChallengeResponse {
                    answers: vec!["proxy-answer".to_string()],
                })
            }),
            ..Default::default()
        };

        let mut handler = ProxyAuthHandler::new(&mut authenticator);
        let response = handler.on_challenge(make_challenge()).await.unwrap();
        assert_eq!(response.answers, vec!["proxy-answer".to_string()]);
    }

    #[test(tokio::test)]
    async fn proxy_auth_handler_delegates_on_verification() {
        let mut authenticator = TestAuthenticator {
            verify: Box::new(|_v| Ok(VerificationResponse { valid: false })),
            ..Default::default()
        };

        let mut handler = ProxyAuthHandler::new(&mut authenticator);
        let response = handler.on_verification(make_verification()).await.unwrap();
        assert!(!response.valid);
    }

    #[test(tokio::test)]
    async fn proxy_auth_handler_delegates_on_info() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        let mut authenticator = TestAuthenticator {
            info: Box::new(move |info| {
                tx.send(info.text.clone()).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let mut handler = ProxyAuthHandler::new(&mut authenticator);
        handler.on_info(make_info()).await.unwrap();
        assert_eq!(rx.recv().unwrap(), "info-text");
    }

    #[test(tokio::test)]
    async fn proxy_auth_handler_delegates_on_error() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        let mut authenticator = TestAuthenticator {
            error: Box::new(move |err| {
                tx.send(err.text.clone()).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let mut handler = ProxyAuthHandler::new(&mut authenticator);
        handler.on_error(make_error()).await.unwrap();
        assert_eq!(rx.recv().unwrap(), "error-text");
    }

    #[test(tokio::test)]
    async fn proxy_auth_handler_propagates_authenticator_errors() {
        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| Err(io::Error::other("init failed"))),
            challenge: Box::new(|_| Err(io::Error::other("challenge failed"))),
            verify: Box::new(|_| Err(io::Error::other("verify failed"))),
            info: Box::new(|_| Err(io::Error::other("info failed"))),
            error: Box::new(|_| Err(io::Error::other("error failed"))),
            start_method: Box::new(|_| Err(io::Error::other("start failed"))),
            finished: Box::new(|| Err(io::Error::other("finished failed"))),
        };

        let mut handler = ProxyAuthHandler::new(&mut authenticator);

        let err = handler
            .on_initialization(Initialization { methods: vec![] })
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "init failed");

        let err = handler
            .on_start_method(StartMethod {
                method: "m".to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "start failed");

        let err = handler.on_finished().await.unwrap_err();
        assert_eq!(err.to_string(), "finished failed");

        let err = handler.on_challenge(make_challenge()).await.unwrap_err();
        assert_eq!(err.to_string(), "challenge failed");

        let err = handler
            .on_verification(make_verification())
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "verify failed");

        let err = handler.on_info(make_info()).await.unwrap_err();
        assert_eq!(err.to_string(), "info failed");

        let err = handler.on_error(make_error()).await.unwrap_err();
        assert_eq!(err.to_string(), "error failed");
    }

    // =======================================================================
    // DynAuthHandler tests
    // =======================================================================

    #[test(tokio::test)]
    async fn dyn_auth_handler_delegates_on_initialization() {
        let mut inner = TestAuthHandler {
            on_initialization: Box::new(|_| {
                Ok(InitializationResponse {
                    methods: vec!["dyn_method".to_string()],
                })
            }),
            ..Default::default()
        };

        let mut handler = DynAuthHandler::new(&mut inner);
        let init = Initialization {
            methods: vec!["a".to_string()],
        };
        let response = handler.on_initialization(init).await.unwrap();
        assert_eq!(response.methods, vec!["dyn_method".to_string()]);
    }

    #[test(tokio::test)]
    async fn dyn_auth_handler_delegates_on_start_method() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        let mut inner = TestAuthHandler {
            on_start_method: Box::new(move |sm| {
                tx.send(sm.method.clone()).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let mut handler = DynAuthHandler::new(&mut inner);
        handler
            .on_start_method(StartMethod {
                method: "target".to_string(),
            })
            .await
            .unwrap();
        assert_eq!(rx.recv().unwrap(), "target");
    }

    #[test(tokio::test)]
    async fn dyn_auth_handler_delegates_on_finished() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        let mut inner = TestAuthHandler {
            on_finished: Box::new(move || {
                tx.send(()).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let mut handler = DynAuthHandler::new(&mut inner);
        handler.on_finished().await.unwrap();
        rx.recv().unwrap();
    }

    #[test(tokio::test)]
    async fn dyn_auth_handler_delegates_on_challenge() {
        let mut inner = TestAuthHandler {
            on_challenge: Box::new(|_| {
                Ok(ChallengeResponse {
                    answers: vec!["dyn-answer".to_string()],
                })
            }),
            ..Default::default()
        };

        let mut handler = DynAuthHandler::new(&mut inner);
        let response = handler.on_challenge(make_challenge()).await.unwrap();
        assert_eq!(response.answers, vec!["dyn-answer".to_string()]);
    }

    #[test(tokio::test)]
    async fn dyn_auth_handler_delegates_on_verification() {
        let mut inner = TestAuthHandler {
            on_verification: Box::new(|_| Ok(VerificationResponse { valid: false })),
            ..Default::default()
        };

        let mut handler = DynAuthHandler::new(&mut inner);
        let response = handler.on_verification(make_verification()).await.unwrap();
        assert!(!response.valid);
    }

    #[test(tokio::test)]
    async fn dyn_auth_handler_delegates_on_info() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        let mut inner = TestAuthHandler {
            on_info: Box::new(move |info| {
                tx.send(info.text.clone()).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let mut handler = DynAuthHandler::new(&mut inner);
        handler.on_info(make_info()).await.unwrap();
        assert_eq!(rx.recv().unwrap(), "info-text");
    }

    #[test(tokio::test)]
    async fn dyn_auth_handler_delegates_on_error() {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        let mut inner = TestAuthHandler {
            on_error: Box::new(move |err| {
                tx.send(err.text.clone()).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let mut handler = DynAuthHandler::new(&mut inner);
        handler.on_error(make_error()).await.unwrap();
        assert_eq!(rx.recv().unwrap(), "error-text");
    }

    #[test(tokio::test)]
    async fn dyn_auth_handler_from_conversion() {
        let mut inner = TestAuthHandler {
            on_challenge: Box::new(|_| {
                Ok(ChallengeResponse {
                    answers: vec!["from-conversion".to_string()],
                })
            }),
            ..Default::default()
        };

        let mut handler = DynAuthHandler::from(&mut inner);
        let response = handler.on_challenge(make_challenge()).await.unwrap();
        assert_eq!(response.answers, vec!["from-conversion".to_string()]);
    }

    #[test(tokio::test)]
    async fn dyn_auth_handler_propagates_inner_errors() {
        let mut inner = TestAuthHandler {
            on_initialization: Box::new(|_| Err(io::Error::other("init err"))),
            on_challenge: Box::new(|_| Err(io::Error::other("challenge err"))),
            on_verification: Box::new(|_| Err(io::Error::other("verify err"))),
            on_info: Box::new(|_| Err(io::Error::other("info err"))),
            on_error: Box::new(|_| Err(io::Error::other("error err"))),
            on_start_method: Box::new(|_| Err(io::Error::other("start err"))),
            on_finished: Box::new(|| Err(io::Error::other("finished err"))),
        };

        let mut handler = DynAuthHandler::from(&mut inner);

        let err = handler
            .on_initialization(Initialization { methods: vec![] })
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "init err");

        let err = handler
            .on_start_method(StartMethod {
                method: "m".to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "start err");

        let err = handler.on_finished().await.unwrap_err();
        assert_eq!(err.to_string(), "finished err");

        let err = handler.on_challenge(make_challenge()).await.unwrap_err();
        assert_eq!(err.to_string(), "challenge err");

        let err = handler
            .on_verification(make_verification())
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "verify err");

        let err = handler.on_info(make_info()).await.unwrap_err();
        assert_eq!(err.to_string(), "info err");

        let err = handler.on_error(make_error()).await.unwrap_err();
        assert_eq!(err.to_string(), "error err");
    }

    // =======================================================================
    // TestAuthHandler tests (verify the test helper itself works correctly)
    // =======================================================================

    #[test(tokio::test)]
    async fn test_auth_handler_default_on_initialization_returns_all_methods() {
        let mut handler = TestAuthHandler::default();
        let init = Initialization {
            methods: vec!["a".to_string(), "b".to_string()],
        };
        let response = handler.on_initialization(init.clone()).await.unwrap();
        assert_eq!(response.methods, init.methods);
    }

    #[test(tokio::test)]
    async fn test_auth_handler_default_on_challenge_echoes_question_text() {
        let mut handler = TestAuthHandler::default();
        let challenge = Challenge {
            questions: vec![Question::new("q1"), Question::new("q2")],
            options: HashMap::new(),
        };
        let response = handler.on_challenge(challenge).await.unwrap();
        assert_eq!(response.answers, vec!["q1".to_string(), "q2".to_string()]);
    }

    #[test(tokio::test)]
    async fn test_auth_handler_default_on_verification_returns_valid() {
        let mut handler = TestAuthHandler::default();
        let response = handler.on_verification(make_verification()).await.unwrap();
        assert!(response.valid);
    }

    #[test(tokio::test)]
    async fn test_auth_handler_default_on_info_succeeds() {
        let mut handler = TestAuthHandler::default();
        handler.on_info(make_info()).await.unwrap();
    }

    #[test(tokio::test)]
    async fn test_auth_handler_default_on_error_succeeds() {
        let mut handler = TestAuthHandler::default();
        handler.on_error(make_error()).await.unwrap();
    }

    #[test(tokio::test)]
    async fn test_auth_handler_default_on_start_method_succeeds() {
        let mut handler = TestAuthHandler::default();
        handler
            .on_start_method(StartMethod {
                method: "m".to_string(),
            })
            .await
            .unwrap();
    }

    #[test(tokio::test)]
    async fn test_auth_handler_default_on_finished_succeeds() {
        let mut handler = TestAuthHandler::default();
        handler.on_finished().await.unwrap();
    }
}
