mod api;
pub use api::*;

mod client;
pub use client::*;

pub mod data;
pub use data::{DistantRequestData, DistantResponseData};

distant_net::router!(TestRouter: u16 -> u32, bool -> String);

mod constants;

mod server;
pub use server::*;
