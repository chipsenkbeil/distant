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
/// This should match the version of this crate such that any significant change to the crate
/// version will also be reflected in this constant that can be used to verify compatibility across
/// the wire.
pub const PROTOCOL_VERSION: SemVer = (
    const_str::parse!(env!("CARGO_PKG_VERSION_MAJOR"), u8),
    const_str::parse!(env!("CARGO_PKG_VERSION_MINOR"), u8),
    const_str::parse!(env!("CARGO_PKG_VERSION_PATCH"), u8),
);
