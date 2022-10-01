use super::{msg::*, Authenticator};
use async_trait::async_trait;
use log::*;
use std::collections::HashMap;
use std::io;

mod none;
mod reauth;
mod static_key;

pub use none::*;
pub use reauth::*;
pub use static_key::*;

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
    /// id of the authentication method that succeeded. Fails if no authentication method succeeds.
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
