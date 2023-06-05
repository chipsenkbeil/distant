#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod api;
mod config;
mod constants;
pub use api::Api;
pub use config::*;
use distant_core::{DistantApi, DistantApiServerHandler};

/// Implementation of [`DistantApiServerHandler`] using [`Api`].
pub type Handler = DistantApiServerHandler<Api, <Api as DistantApi>::LocalData>;

/// Initializes a new [`Handler`].
pub fn new_handler(config: Config) -> std::io::Result<Handler> {
    Ok(Handler::new(Api::initialize(config)?))
}
