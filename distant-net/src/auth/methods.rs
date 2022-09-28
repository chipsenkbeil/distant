use super::{msg::*, Authenticator};
use crate::HeapSecretKey;
use async_trait::async_trait;
use log::*;
use std::collections::HashMap;
use std::io;

/// Supports authenticating using a variety of methods
pub struct Verifier {
    methods: HashMap<&'static str, Box<dyn AuthenticationMethod>>,
}

impl Verifier {
    pub fn new<I>(methods: I) -> Self
    where
        I: IntoIterator<Item = Box<dyn AuthenticationMethod>>,
    {
        let mut m = HashMap::new();

        for method in methods {
            m.insert(method.id(), method);
        }

        Self { methods: m }
    }

    /// Creates a verifier with no methods.
    pub fn empty() -> Self {
        Self {
            methods: HashMap::new(),
        }
    }

    /// Returns an iterator over the ids of the methods supported by the verifier
    pub fn methods(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.methods.keys().copied()
    }

    /// Attempts to verify by submitting challenges using the `authenticator` provided. Returns the
    /// id of the authentication method that succeeded.
    pub async fn verify(&self, authenticator: &mut dyn Authenticator) -> io::Result<&'static str> {
        // Initiate the process to get methods to use
        let response = authenticator
            .initialize(Initialization {
                methods: self.methods.keys().map(ToString::to_string).collect(),
            })
            .await?;

        for method in response.methods {
            match self.methods.get(method.as_str()) {
                Some(method) => {
                    if method.authenticate(authenticator).await.is_ok() {
                        authenticator.finished().await?;
                        return Ok(method.id());
                    }
                }
                None => {
                    trace!("Skipping authentication {method} as it is not available or supported");
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "No authentication method succeeded",
        ))
    }
}

/// Represents an interface to authenticate using some method
#[async_trait]
pub trait AuthenticationMethod: Send + Sync {
    /// Returns a unique id to distinguish the method from other methods
    fn id(&self) -> &'static str;

    /// Performs authentication using the `authenticator` to submit challenges and other
    /// information based on the authentication method
    async fn authenticate(&self, authenticator: &mut dyn Authenticator) -> io::Result<()>;
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
    fn id(&self) -> &'static str {
        "none"
    }

    async fn authenticate(&self, _: &mut dyn Authenticator) -> io::Result<()> {
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
    fn id(&self) -> &'static str {
        "static_key"
    }

    async fn authenticate(&self, authenticator: &mut dyn Authenticator) -> io::Result<()> {
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
    fn id(&self) -> &'static str {
        "reauthentication"
    }

    async fn authenticate(&self, authenticator: &mut dyn Authenticator) -> io::Result<()> {
        self.method.authenticate(authenticator).await
    }
}
