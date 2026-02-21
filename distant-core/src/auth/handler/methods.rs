use std::io;

use async_trait::async_trait;

use crate::auth::msg::{
    Challenge, ChallengeResponse, Error, Info, Verification, VerificationResponse,
};

/// Interface for a handler of authentication requests for a specific authentication method.
#[async_trait]
pub trait AuthMethodHandler: Send {
    /// Callback when a challenge is received, returning answers to the given questions.
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse>;

    /// Callback when a verification request is received, returning true if approvided or false if
    /// unapproved.
    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse>;

    /// Callback when information is received. To fail, return an error from this function.
    async fn on_info(&mut self, info: Info) -> io::Result<()>;

    /// Callback when an error is received. Regardless of the result returned, this will terminate
    /// the authenticator. In the situation where a custom error would be preferred, have this
    /// callback return an error.
    async fn on_error(&mut self, error: Error) -> io::Result<()>;
}

mod prompt;
pub use prompt::*;

mod static_key;
pub use static_key::*;
