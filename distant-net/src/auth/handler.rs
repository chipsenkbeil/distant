use super::msg::*;
use async_trait::async_trait;
use std::io;

/// Interface for a handler of authentication requests
#[async_trait]
pub trait AuthHandler {
    /// Callback when a challenge is received, returning answers to the given questions.
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<Vec<String>>;

    /// Callback when a verification request is received, returning true if approvided or false if
    /// unapproved.
    async fn on_verification(&mut self, verification: Verification) -> io::Result<bool>;

    /// Callback when authentication starts for a specific method
    #[allow(unused_variables)]
    async fn on_start(&mut self, start: AuthenticationStart) -> io::Result<()> {
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
