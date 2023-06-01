#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

mod authenticator;
mod handler;
mod methods;
pub mod msg;

pub use authenticator::*;
pub use handler::*;
pub use methods::*;

#[cfg(any(test, feature = "tests"))]
pub mod tests {
    pub use crate::{TestAuthHandler, TestAuthenticator};
}
