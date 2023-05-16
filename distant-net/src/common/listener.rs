use std::io;

use async_trait::async_trait;

mod mapped;
pub use mapped::*;

mod mpsc;
pub use mpsc::*;

mod oneshot;
pub use oneshot::*;

mod tcp;
pub use tcp::*;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::*;

/// Represents a type that has a listen interface for receiving raw streams
#[async_trait]
pub trait Listener: Send + Sync {
    type Output;

    async fn accept(&mut self) -> io::Result<Self::Output>;
}
