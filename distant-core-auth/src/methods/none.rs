use std::io;

use async_trait::async_trait;

use crate::authenticator::Authenticator;
use crate::methods::AuthenticationMethod;

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

#[async_trait]
impl AuthenticationMethod for NoneAuthenticationMethod {
    fn id(&self) -> &'static str {
        Self::ID
    }

    async fn authenticate(&self, _: &mut dyn Authenticator) -> io::Result<()> {
        Ok(())
    }
}
