use async_trait::async_trait;
use std::io;

mod router;

mod raw;
pub use raw::*;

mod typed;
pub use typed::*;

mod untyped;
pub use untyped::*;

pub use tokio::io::{Interest, Ready};

/// Interface representing a connection that is reconnectable
#[async_trait]
pub trait Reconnectable {
    /// Attempts to reconnect an already-established connection
    async fn reconnect(&mut self) -> io::Result<()>;
}
