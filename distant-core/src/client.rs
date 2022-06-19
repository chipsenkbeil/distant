use crate::{DistantMsg, DistantRequestData, DistantResponseData};
use distant_net::{Channel, Client};

mod ext;
mod lsp;
mod process;
mod watcher;

pub type DistantClient = Client<DistantMsg<DistantRequestData>, DistantMsg<DistantResponseData>>;
pub type DistantChannel = Channel<DistantMsg<DistantRequestData>, DistantMsg<DistantResponseData>>;

pub use ext::*;
pub use lsp::*;
pub use process::*;
pub use watcher::*;
