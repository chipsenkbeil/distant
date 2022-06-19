use crate::{DistantMsg, DistantRequestData, DistantResponseData};
use distant_net::{Channel, Client};

mod lsp;
mod process;
mod session;
mod watcher;

pub type DistantClient = Client<DistantMsg<DistantRequestData>, DistantMsg<DistantResponseData>>;
pub type DistantChannel = Channel<DistantMsg<DistantRequestData>, DistantMsg<DistantResponseData>>;

pub use lsp::*;
pub use process::*;
pub use session::*;
pub use watcher::*;
