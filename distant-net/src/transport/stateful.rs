use super::{FramedTransport, HeapSecretKey, Reconnectable, Transport};
use async_trait::async_trait;
use std::{
    io,
    ops::{Deref, DerefMut},
};

/// Internal state for our transport
#[derive(Clone, Debug)]
enum State {
    /// Transport is not authenticated and has not begun the process of authenticating
    NotAuthenticated,

    /// Transport is in the state of currently authenticating, either by issuing challenges or
    /// responding with answers to challenges
    Authenticating,

    /// Transport has finished authenticating successfully
    Authenticated {
        /// Unique key that marks the transport as authenticated for use in shortcutting
        /// authentication when the transport reconnects. This is NOT the key used for encryption
        /// and is instead meant to be shared (secretly) between transports that are aware of a
        /// previously-successful authentication.
        key: HeapSecretKey,
    },
}

/// Represents an stateful [`FramedTransport`] that is capable of performing authentication with
/// another [`FramedTransport`] in order to properly encrypt messages by deriving an appropriate
/// encryption codec. When authenticated, reconnecting will skip authentication unless the
/// transport on the other side declines the authenticated state.
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

    /// Performs authentication with the other side, moving the state to be authenticated.
    ///
    /// NOTE: Does nothing if already authenticated!
    pub async fn authenticate(&mut self) -> io::Result<()> {
        if self.is_authenticated() {
            return Ok(());
        }

        todo!();
    }

    /// Returns true if has not started the authentication process
    ///
    /// NOTE: This will return false if in the process of authenticating, but not finished! To
    ///       check if not authenticated or actively authenticating, use ![`is_authenticated`].
    ///
    /// [`is_authenticated`]: StatefulFramedTransport::is_authenticated
    pub fn is_not_authenticated(&self) -> bool {
        matches!(self.state, State::NotAuthenticated)
    }

    /// Returns true if actively authenticating
    pub fn is_authenticating(&self) -> bool {
        matches!(self.state, State::Authenticating)
    }

    /// Returns true if has authenticated successfully
    pub fn is_authenticated(&self) -> bool {
        matches!(self.state, State::Authenticated { .. })
    }
}

impl<T, const CAPACITY: usize> Deref for StatefulFramedTransport<T, CAPACITY> {
    type Target = FramedTransport<T, CAPACITY>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T, const CAPACITY: usize> DerefMut for StatefulFramedTransport<T, CAPACITY> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[async_trait]
impl<T, const CAPACITY: usize> Reconnectable for StatefulFramedTransport<T, CAPACITY>
where
    T: Transport + Send + Sync,
{
    async fn reconnect(&mut self) -> io::Result<()> {
        match self.state {
            // If not authenticated or in the process of authenticating, we simply perform a raw
            // reconnect and reset to not being authenticated
            State::NotAuthenticated | State::Authenticating => {
                self.state = State::NotAuthenticated;
                Reconnectable::reconnect(&mut self.inner).await
            }

            // If authenticated, we perform a reconnect followed by re-authentication using our
            // previously-acquired key to skip the need to do another authentication. Note that
            // this can still change the underlying codec used by the transport if an alternative
            // compression or encryption codec is picked.
            State::Authenticated { key, .. } => {
                Reconnectable::reconnect(&mut self.inner).await?;

                todo!("do handshake with key");
            }
        }
    }
}
