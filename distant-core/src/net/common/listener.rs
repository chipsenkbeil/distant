use std::future::Future;
use std::io;

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
pub trait Listener: Send + Sync {
    type Output;

    fn accept(&mut self) -> impl Future<Output = io::Result<Self::Output>> + Send;
}
