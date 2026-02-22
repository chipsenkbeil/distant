use std::future::Future;
use std::io;
use std::pin::Pin;

use crate::auth::authenticator::Authenticator;
use crate::auth::methods::AuthenticationMethod;

/// Authenticaton method that skips authentication and approves anything.
#[derive(Clone, Debug)]
pub struct NoneAuthenticationMethod;

impl NoneAuthenticationMethod {
    pub const ID: &str = "none";

    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoneAuthenticationMethod {
    #[inline]
    fn default() -> Self {
        Self
    }
}

impl AuthenticationMethod for NoneAuthenticationMethod {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn authenticate<'a>(
        &'a self,
        _: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::authenticator::TestAuthenticator;
    use crate::auth::methods::AuthenticationMethod;

    #[test]
    fn id_constant_is_none() {
        assert_eq!(NoneAuthenticationMethod::ID, "none");
    }

    #[test]
    fn new_creates_instance() {
        let _method = NoneAuthenticationMethod::new();
    }

    #[test]
    fn default_creates_instance() {
        let _method = NoneAuthenticationMethod;
    }

    #[test]
    fn id_returns_none() {
        let method = NoneAuthenticationMethod::new();
        assert_eq!(method.id(), "none");
    }

    #[test_log::test(tokio::test)]
    async fn authenticate_returns_ok() {
        let method = NoneAuthenticationMethod::new();
        let mut authenticator = TestAuthenticator::default();
        let result = method.authenticate(&mut authenticator).await;
        assert!(result.is_ok());
    }
}
