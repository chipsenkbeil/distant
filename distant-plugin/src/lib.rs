#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

pub mod api;
pub mod client;
pub mod common;
pub mod handlers;

pub use distant_core_auth as auth;
pub use distant_core_protocol as protocol;
