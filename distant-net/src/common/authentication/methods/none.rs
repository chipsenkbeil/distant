use std::io;

use async_trait::async_trait;

use super::{AuthenticationMethod, Authenticator};

/// Authenticaton method for a static secret key
#[derive(Clone, Debug)]
pub struct NoneAuthenticationMethod;

impl NoneAuthenticationMethod {
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
        "none"
    }

    async fn authenticate(&self, _: &mut dyn Authenticator) -> io::Result<()> {
        Ok(())
    }
}
