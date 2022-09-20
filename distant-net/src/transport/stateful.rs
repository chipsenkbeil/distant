use super::{FramedTransport, HeapSecretKey, Reconnectable, Transport};
use async_trait::async_trait;
use std::io;

mod handshake;
pub use handshake::*;

#[derive(Clone, Debug)]
enum State {
    NotAuthenticated,
    Authenticated {
        key: HeapSecretKey,
        handshake_options: HandshakeOptions,
    },
}

/// Represents an stateful framed transport that is capable of peforming handshakes and
/// reconnecting using an authenticated state
#[derive(Clone, Debug)]
pub struct StatefulFramedTransport<T, const CAPACITY: usize> {
    inner: FramedTransport<T, CAPACITY>,
    state: State,
}

impl<T, const CAPACITY: usize> StatefulFramedTransport<T, CAPACITY> {
    /// Creates a new stateful framed transport that is not yet authenticated
    pub fn new(inner: FramedTransport<T, CAPACITY>) -> Self {
        Self {
            inner,
            state: State::NotAuthenticated,
        }
    }

    /// Performs an authentication handshake, moving the state to be authenticated.
    ///
    /// Does nothing if already authenticated
    pub async fn authenticate(&mut self, handshake_options: HandshakeOptions) -> io::Result<()> {
        if self.is_authenticated() {
            return Ok(());
        }

        todo!();
    }

    /// Returns true if in an authenticated state
    pub fn is_authenticated(&self) -> bool {
        matches!(self.state, State::Authenticated { .. })
    }

    /// Returns a reference to the [`HandshakeOptions`] used during authentication. Returns `None`
    /// if not authenticated.
    pub fn handshake_options(&self) -> Option<&HandshakeOptions> {
        match &self.state {
            State::NotAuthenticated => None,
            State::Authenticated {
                handshake_options, ..
            } => Some(handshake_options),
        }
    }
}

#[async_trait]
impl<T, const CAPACITY: usize> Reconnectable for StatefulFramedTransport<T, CAPACITY>
where
    T: Transport + Send + Sync,
{
    async fn reconnect(&mut self) -> io::Result<()> {
        match self.state {
            // If not authenticated, we simply perform a raw reconnect
            State::NotAuthenticated => Reconnectable::reconnect(&mut self.inner).await,

            // If authenticated, we perform a reconnect followed by re-authentication using our
            // previously-derived key to skip the need to do another authentication
            State::Authenticated { key, .. } => {
                Reconnectable::reconnect(&mut self.inner).await?;

                todo!("do handshake with key");
            }
        }
    }
}
