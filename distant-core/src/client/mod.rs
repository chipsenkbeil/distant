mod lsp;
mod process;
mod session;
mod utils;
mod watcher;

pub type DistantSession =
    distant_net::Client<Vec<crate::DistantRequestData>, Vec<crate::DistantResponseData>>;
pub type DistantChannel =
    distant_net::Channel<Vec<crate::DistantRequestData>, Vec<crate::DistantResponseData>>;

pub use lsp::*;
pub use process::*;
pub use session::*;
pub use watcher::*;
