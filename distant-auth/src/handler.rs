use std::collections::HashMap;
use std::fmt::Display;
use std::io;

use async_trait::async_trait;

use crate::authenticator::Authenticator;
use crate::msg::*;

mod methods;
pub use methods::*;

/// Interface for a handler of authentication requests for all methods.
#[async_trait]
pub trait AuthHandler: AuthMethodHandler + Send {
    /// Callback when authentication is beginning, providing available authentication methods and
    /// returning selected authentication methods to pursue.
    async fn on_initialization(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        Ok(InitializationResponse {
            methods: initialization.methods,
        })
    }

    /// Callback when authentication starts for a specific method.
    #[allow(unused_variables)]
    async fn on_start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        Ok(())
    }

    /// Callback when authentication is finished and no more requests will be received.
    async fn on_finished(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Dummy implementation of [`AuthHandler`] where any challenge or verification request will
/// instantly fail.
pub struct DummyAuthHandler;

#[async_trait]
impl AuthHandler for DummyAuthHandler {}

#[async_trait]
impl AuthMethodHandler for DummyAuthHandler {
    async fn on_challenge(&mut self, _: Challenge) -> io::Result<ChallengeResponse> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    async fn on_verification(&mut self, _: Verification) -> io::Result<VerificationResponse> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    async fn on_info(&mut self, _: Info) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    async fn on_error(&mut self, _: Error) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }
}

/// Implementation of [`AuthHandler`] that uses the same [`AuthMethodHandler`] for all methods.
pub struct SingleAuthHandler(Box<dyn AuthMethodHandler>);

impl SingleAuthHandler {
    pub fn new<T: AuthMethodHandler + 'static>(method_handler: T) -> Self {
        Self(Box::new(method_handler))
    }
}

#[async_trait]
impl AuthHandler for SingleAuthHandler {}

#[async_trait]
impl AuthMethodHandler for SingleAuthHandler {
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        self.0.on_challenge(challenge).await
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
        self.0.on_verification(verification).await
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        self.0.on_info(info).await
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        self.0.on_error(error).await
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
        self.get_mut_active_method_handler().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, format!("No active handler for {id}"))
        })
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

#[async_trait]
impl AuthHandler for AuthHandlerMap {
    async fn on_initialization(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        let methods = initialization
            .methods
            .into_iter()
            .filter(|method| self.map.contains_key(method.as_str()))
            .collect();

        Ok(InitializationResponse { methods })
    }

    async fn on_start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        self.set_active_id(start_method.method);
        Ok(())
    }

    async fn on_finished(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl AuthMethodHandler for AuthHandlerMap {
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        let handler = self.get_mut_active_method_handler_or_error()?;
        handler.on_challenge(challenge).await
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
        let handler = self.get_mut_active_method_handler_or_error()?;
        handler.on_verification(verification).await
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        let handler = self.get_mut_active_method_handler_or_error()?;
        handler.on_info(info).await
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        let handler = self.get_mut_active_method_handler_or_error()?;
        handler.on_error(error).await
    }
}

/// Implementation of [`AuthHandler`] that redirects all requests to an [`Authenticator`].
pub struct ProxyAuthHandler<'a>(&'a mut dyn Authenticator);

impl<'a> ProxyAuthHandler<'a> {
    pub fn new(authenticator: &'a mut dyn Authenticator) -> Self {
        Self(authenticator)
    }
}

#[async_trait]
impl<'a> AuthHandler for ProxyAuthHandler<'a> {
    async fn on_initialization(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        Authenticator::initialize(self.0, initialization).await
    }

    async fn on_start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        Authenticator::start_method(self.0, start_method).await
    }

    async fn on_finished(&mut self) -> io::Result<()> {
        Authenticator::finished(self.0).await
    }
}

#[async_trait]
impl<'a> AuthMethodHandler for ProxyAuthHandler<'a> {
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        Authenticator::challenge(self.0, challenge).await
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
        Authenticator::verify(self.0, verification).await
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        Authenticator::info(self.0, info).await
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        Authenticator::error(self.0, error).await
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

#[async_trait]
impl<'a> AuthHandler for DynAuthHandler<'a> {
    async fn on_initialization(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        self.0.on_initialization(initialization).await
    }

    async fn on_start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        self.0.on_start_method(start_method).await
    }

    async fn on_finished(&mut self) -> io::Result<()> {
        self.0.on_finished().await
    }
}

#[async_trait]
impl<'a> AuthMethodHandler for DynAuthHandler<'a> {
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        self.0.on_challenge(challenge).await
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
        self.0.on_verification(verification).await
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        self.0.on_info(info).await
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        self.0.on_error(error).await
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
#[async_trait]
impl AuthHandler for TestAuthHandler {
    async fn on_initialization(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        (self.on_initialization)(initialization)
    }

    async fn on_start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        (self.on_start_method)(start_method)
    }

    async fn on_finished(&mut self) -> io::Result<()> {
        (self.on_finished)()
    }
}

#[cfg(any(test, feature = "tests"))]
#[async_trait]
impl AuthMethodHandler for TestAuthHandler {
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        (self.on_challenge)(challenge)
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
        (self.on_verification)(verification)
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        (self.on_info)(info)
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        (self.on_error)(error)
    }
}
