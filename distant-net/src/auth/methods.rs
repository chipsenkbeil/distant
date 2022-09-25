use super::{msg::*, Authenticator};
use crate::HeapSecretKey;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::io;

/// Represents the type of authentication method to use
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MethodType {
    /// Indicates no authentication, which means this will always succeed
    None,

    /// Indicates that a static key is being used for authentication
    StaticKey,

    /// Indicates that re-authentication is being employed (using specialized key)
    Reauthentication,

    /// When the method is unknown (happens when other side is unaware of the method)
    #[serde(other)]
    Unknown,
}

/// Represents an interface to authenticate using some method
#[async_trait]
pub trait AuthenticationMethod {
    // TODO: add a unique id method and update below method to take dyn ref so it can be boxed.
    // that way, we can pass to server a collection of boxed methods
    async fn authenticate<A: Authenticator>(&self, authenticator: &mut A) -> io::Result<()>;
}

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
    async fn authenticate<A: Authenticator>(&self, _: &mut A) -> io::Result<()> {
        Ok(())
    }
}

/// Authenticaton method for a static secret key
#[derive(Clone, Debug)]
pub struct StaticKeyAuthenticationMethod {
    key: HeapSecretKey,
}

impl StaticKeyAuthenticationMethod {
    #[inline]
    pub fn new(key: impl Into<HeapSecretKey>) -> Self {
        Self { key: key.into() }
    }
}

#[async_trait]
impl AuthenticationMethod for StaticKeyAuthenticationMethod {
    async fn authenticate<A: Authenticator>(&self, authenticator: &mut A) -> io::Result<()> {
        let response = authenticator
            .challenge(Challenge {
                questions: vec![Question::new("key")],
                options: Default::default(),
            })
            .await?;

        if response.answers.is_empty() {
            let x = Error::fatal("missing answer");
            authenticator.error(x.clone()).await?;
            return Err(x.into_io_permission_denied());
        } else if response.answers.len() > 1 {
            authenticator
                .error(Error::non_fatal("more than one answer, picking first"))
                .await?;
        }

        match response
            .answers
            .into_iter()
            .next()
            .unwrap()
            .parse::<HeapSecretKey>()
        {
            Ok(key) if key == self.key => Ok(()),
            _ => {
                let x = Error::fatal("answer not a valid key");
                authenticator.error(x.clone()).await?;
                Err(x.into_io_permission_denied())
            }
        }
    }
}

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
    async fn authenticate<A: Authenticator>(&self, authenticator: &mut A) -> io::Result<()> {
        self.method.authenticate(authenticator).await
    }
}
