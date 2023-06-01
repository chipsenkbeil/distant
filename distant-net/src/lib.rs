#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod authentication;
pub mod client;
pub mod common;
pub mod manager;
pub mod server;

pub use client::{Client, ReconnectStrategy};
/// Authentication functionality tied to network operations.
pub use distant_auth as auth;
pub use server::Server;
pub use {log, paste};
