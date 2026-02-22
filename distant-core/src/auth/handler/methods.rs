use std::future::Future;
use std::io;
use std::pin::Pin;

use crate::auth::msg::{
    Challenge, ChallengeResponse, Error, Info, Verification, VerificationResponse,
};

/// Interface for a handler of authentication requests for a specific authentication method.
pub trait AuthMethodHandler: Send {
    /// Callback when a challenge is received, returning answers to the given questions.
    fn on_challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>>;

    /// Callback when a verification request is received, returning true if approvided or false if
    /// unapproved.
    fn on_verification<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>>;

    /// Callback when information is received. To fail, return an error from this function.
    fn on_info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

    /// Callback when an error is received. Regardless of the result returned, this will terminate
    /// the authenticator. In the situation where a custom error would be preferred, have this
    /// callback return an error.
    fn on_error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;
}

mod prompt;
pub use prompt::*;

mod static_key;
pub use static_key::*;
