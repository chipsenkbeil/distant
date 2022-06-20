mod api;
pub use api::*;

mod client;
pub use client::*;

pub mod data;
pub use data::{DistantRequestData, DistantResponseData};

distant_net::router!(TestRouter: u16 -> u32, bool -> String);

mod constants;

/// Re-export of `distant-net` as `net`
pub use distant_net as net;
