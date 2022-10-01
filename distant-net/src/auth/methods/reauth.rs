use super::{AuthenticationMethod, Authenticator, StaticKeyAuthenticationMethod};
use crate::HeapSecretKey;
use async_trait::async_trait;
use std::io;

/// Authenticaton method for reauthentication
#[derive(Clone, Debug)]
pub struct ReauthenticationMethod {
    method: StaticKeyAuthenticationMethod,
}

impl ReauthenticationMethod {
    #[inline]
    pub fn new(key: impl Into<HeapSecretKey>) -> Self {
        Self {
            method: StaticKeyAuthenticationMethod::new(key),
        }
    }
}

#[async_trait]
impl AuthenticationMethod for ReauthenticationMethod {
    fn id(&self) -> &'static str {
        "reauthentication"
    }

    async fn authenticate(&self, authenticator: &mut dyn Authenticator) -> io::Result<()> {
        self.method.authenticate(authenticator).await
    }
}
