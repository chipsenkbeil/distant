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
