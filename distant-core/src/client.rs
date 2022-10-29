use crate::{DistantMsg, DistantRequestData, DistantResponseData};
use distant_net::{client::Channel, Client};

mod ext;
mod lsp;
mod process;
mod searcher;
mod watcher;

/// Represents a [`Client`] that communicates using the distant protocol
pub type DistantClient = Client<DistantMsg<DistantRequestData>, DistantMsg<DistantResponseData>>;

/// Represents a [`Channel`] that communicates using the distant protocol
pub type DistantChannel = Channel<DistantMsg<DistantRequestData>, DistantMsg<DistantResponseData>>;

pub use ext::*;
pub use lsp::*;
pub use process::*;
pub use searcher::*;
pub use watcher::*;
