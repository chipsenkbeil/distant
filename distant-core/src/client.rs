use distant_net::client::Channel;
use distant_net::Client;

use crate::protocol;

mod ext;
mod lsp;
mod process;
mod searcher;
mod watcher;

/// Represents a [`Client`] that communicates using the distant protocol
pub type DistantClient =
    Client<protocol::Msg<protocol::Request>, protocol::Msg<protocol::Response>>;

/// Represents a [`Channel`] that communicates using the distant protocol
pub type DistantChannel =
    Channel<protocol::Msg<protocol::Request>, protocol::Msg<protocol::Response>>;

pub use ext::*;
pub use lsp::*;
pub use process::*;
pub use searcher::*;
pub use watcher::*;
