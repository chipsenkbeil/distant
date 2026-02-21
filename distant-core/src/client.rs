use crate::protocol;

mod ext;
mod lsp;
mod process;
mod searcher;
mod watcher;

/// Represents a [`crate::net::Client`] that communicates using the distant protocol
pub type Client =
    crate::net::Client<protocol::Msg<protocol::Request>, protocol::Msg<protocol::Response>>;

/// Represents a [`crate::net::client::Channel`] that communicates using the distant protocol
pub type Channel = crate::net::client::Channel<
    protocol::Msg<protocol::Request>,
    protocol::Msg<protocol::Response>,
>;

pub use ext::*;
pub use lsp::*;
pub use process::*;
pub use searcher::*;
pub use watcher::*;
