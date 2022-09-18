use super::{HandshakeClientChoice, HandshakeServerOptions};
use std::fmt;

/// Callback invoked when a client receives server options during a handshake
pub struct OnHandshakeClientChoice(
    pub(super) Box<dyn Fn(HandshakeServerOptions) -> HandshakeClientChoice>,
);

impl OnHandshakeClientChoice {
    /// Wraps a function `f` as a callback
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(HandshakeServerOptions) -> HandshakeClientChoice,
    {
        Self(Box::new(f))
    }
}

impl<F> From<F> for OnHandshakeClientChoice
where
    F: Fn(HandshakeServerOptions) -> HandshakeClientChoice,
{
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

impl fmt::Debug for OnHandshakeClientChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnHandshakeClientChoice").finish()
    }
}

impl Default for OnHandshakeClientChoice {
    /// Implements choice selection that picks first available of encryption and nothing of
    /// compression
    fn default() -> Self {
        Self::new(|options| HandshakeClientChoice {
            compression: None,
            compression_level: None,
            encryption: options.encryption.first().copied(),
        })
    }
}
