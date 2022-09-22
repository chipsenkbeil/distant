use super::{FramedTransport, HeapSecretKey, Reconnectable, Transport};
use async_trait::async_trait;
use std::{
    collections::HashMap,
    io,
    ops::{Deref, DerefMut},
};

mod data;
pub use data::*;

/// Internal state for a singular transport
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
        /// authentication when a transport needs to reconnect. This is NOT the key used for
        /// encryption and is instead meant to be shared (secretly) between client-server that are
        /// aware of a previously-successful authentication.
        key: HeapSecretKey,
    },
}

/// Represents a stateful authenticator that is capable of performing authentication with
/// another [`Authenticator`] by communicating through a [`FramedTransport`].
///
/// ### Details
///
/// The authenticator manages a mapping of `ClientId` -> `Key` upon successful authentication which
/// can be used to verify re-authentication without needing to perform full authentication again.
/// This is particularly useful in re-connecting a `FramedTransport` post-handshake after a network
/// disruption.
#[derive(Clone, Debug)]
pub struct Authenticator {
    authenticated: HashMap<String, State>,
}

impl Authenticator {
    pub fn new() -> Self {
        Self {
            authenticated: HashMap::new(),
        }
    }

    /// Performs authentication with the other side, moving the state to be authenticated.
    pub async fn authenticate<T, const CAPACITY: usize>(
        &mut self,
        transport: &mut FramedTransport<T, CAPACITY>,
        authentication: Authentication,
    ) -> io::Result<()> {
        if self.is_authenticated() {
            return Ok(());
        }

        todo!();
    }

    /// Clears out any tracked clients
    pub fn clear(&mut self) {
        self.authenticated.clear();
    }
}

impl Default for Authenticator {
    fn default() -> Self {
        Self::new()
    }
}
