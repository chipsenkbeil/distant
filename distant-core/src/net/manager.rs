mod client;
mod data;
mod server;

pub use client::*;
pub use data::*;
pub use server::*;

use crate::net::common::Version;

/// Represents the version associated with the manager's protocol.
pub const PROTOCOL_VERSION: Version = Version::new(
    const_str::parse!(env!("CARGO_PKG_VERSION_MAJOR"), u64),
    const_str::parse!(env!("CARGO_PKG_VERSION_MINOR"), u64),
    const_str::parse!(env!("CARGO_PKG_VERSION_PATCH"), u64),
);
