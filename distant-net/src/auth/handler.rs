use super::data::*;
use async_trait::async_trait;
use std::{collections::HashMap, io};

/// Interface for a handler of authentication requests
#[async_trait]
pub trait AuthHandler {
    /// Callback when a challenge is received, returning answers to the given questions.
    async fn on_challenge(
        &mut self,
        questions: Vec<AuthQuestion>,
        options: HashMap<String, String>,
    ) -> io::Result<Vec<String>>;

    /// Callback when a verification request is received, returning true if approvided or false if
    /// unapproved.
    async fn on_verify(&mut self, kind: AuthVerifyKind, text: String) -> io::Result<bool>;

    /// Callback when authentication is finished and no more requests will be received
    async fn on_finished(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Callback when information is received. To fail, return an error from this function.
    #[allow(unused_variables)]
    async fn on_info(&mut self, text: String) -> io::Result<()> {
        Ok(())
    }

    /// Callback when an error is received. Regardless of the result returned, this will terminate
    /// the authenticator. In the situation where a custom error would be preferred, have this
    /// callback return an error.
    #[allow(unused_variables)]
    async fn on_error(&mut self, kind: AuthErrorKind, text: &str) -> io::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl<H: AuthHandler + Send> AuthHandler for &mut H {
    async fn on_challenge(
        &mut self,
        questions: Vec<AuthQuestion>,
        options: HashMap<String, String>,
    ) -> io::Result<Vec<String>> {
        AuthHandler::on_challenge(self, questions, options).await
    }

    async fn on_verify(&mut self, kind: AuthVerifyKind, text: String) -> io::Result<bool> {
        AuthHandler::on_verify(self, kind, text).await
    }

    async fn on_finished(&mut self) -> io::Result<()> {
        AuthHandler::on_finished(self).await
    }

    async fn on_info(&mut self, text: String) -> io::Result<()> {
        AuthHandler::on_info(self, text).await
    }

    async fn on_error(&mut self, kind: AuthErrorKind, text: &str) -> io::Result<()> {
        AuthHandler::on_error(self, kind, text).await
    }
}
