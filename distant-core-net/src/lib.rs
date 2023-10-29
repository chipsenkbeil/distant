#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod any;
mod authentication;
mod connection;
mod key;
mod keychain;
mod listener;
mod packet;
mod port;
mod transport;
pub(crate) mod utils;
mod version;

pub use any::*;
pub use connection::*;
pub use key::*;
pub use keychain::*;
pub use listener::*;
pub use packet::*;
pub use port::*;
pub use transport::*;
pub use version::*;

/// Authentication functionality tied to network operations.
pub use distant_core_auth as auth;
pub use {log, paste};
