#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod api;
pub use api::*;

mod client;
pub use client::*;

mod credentials;
pub use credentials::*;

mod constants;
mod serde_str;

/// Network functionality.
pub use distant_net as net;
/// Protocol structures.
pub use distant_protocol as protocol;
