mod authentication;
pub mod client;
pub mod common;
pub mod manager;
pub mod server;

pub use client::{Client, ReconnectStrategy};
pub use server::Server;
pub use {log, paste};

/// Authentication functionality tied to network operations.
pub use distant_auth as auth;
