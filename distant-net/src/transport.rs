use std::marker::Unpin;
use tokio::io::{AsyncRead, AsyncWrite};

/// Interface representing a bidirectional transport interface
pub trait Transport: AsyncRead + AsyncWrite + Unpin {
    type ReadHalf: AsyncRead + Send + Unpin + 'static;
    type WriteHalf: AsyncWrite + Send + Unpin + 'static;

    /// Splits this stream into receiving and sending halves
    fn into_split(self) -> (Self::ReadHalf, Self::WriteHalf);
}

mod router;

mod framed;
pub use framed::*;

mod inmemory;
pub use inmemory::*;

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
