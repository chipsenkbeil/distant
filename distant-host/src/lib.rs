#![doc = include_str!("../README.md")]
#![allow(dead_code)] // Allow unused trait methods and structs for API completeness

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod api;
mod config;
mod constants;
pub use api::Api;
pub use config::*;
use distant_core::ApiServerHandler;

/// Implementation of [`ApiServerHandler`] using [`Api`].
pub type Handler = ApiServerHandler<Api>;

/// Initializes a new [`Handler`].
pub fn new_handler(config: Config) -> std::io::Result<Handler> {
    Ok(Handler::new(Api::initialize(config)?))
}
