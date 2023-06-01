#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod common;
mod msg;
mod request;
mod response;
mod utils;

pub use common::*;
pub use msg::*;
pub use request::*;
pub use response::*;

/// Protocol version indicated by the tuple of (major, minor, patch).
///
/// This is different from the crate version, which matches that of the complete suite of distant
/// crates. Rather, this verison is used to provide stability indicators when the protocol itself
/// changes across crate versions.
pub const PROTOCOL_VERSION: SemVer = (0, 1, 0);
