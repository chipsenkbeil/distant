#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod api;
mod constants;
pub use api::LocalDistantApi;
use distant_core::{DistantApi, DistantApiServerHandler};

/// Implementation of [`DistantApiServerHandler`] using [`LocalDistantApi`].
pub type LocalDistantApiServerHandler =
    DistantApiServerHandler<LocalDistantApi, <LocalDistantApi as DistantApi>::LocalData>;

/// Initializes a new [`LocalDistantApiServerHandler`].
pub fn initialize_handler() -> std::io::Result<LocalDistantApiServerHandler> {
    Ok(LocalDistantApiServerHandler::new(
        LocalDistantApi::initialize()?,
    ))
}
