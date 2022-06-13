use async_trait::async_trait;
use std::{io, marker::Unpin};
use tokio::io::{AsyncRead, AsyncWrite};

/// Interface representing a transport of raw bytes into and out of the system
pub trait RawTransport:
    RawTransportRead + RawTransportWrite + IntoSplit<Left = Self::ReadHalf, Right = Self::WriteHalf>
{
    type ReadHalf: RawTransportRead;
    type WriteHalf: RawTransportWrite;
}

/// Interface representing a transport of raw bytes into the system
pub trait RawTransportRead: AsyncRead + Send + Unpin {}

/// Interface representing a transport of raw bytes out of the system
pub trait RawTransportWrite: AsyncWrite + Send + Unpin {}

/// Interface to split something into left and right sides
pub trait IntoSplit {
    type Left;
    type Right;

    fn into_split(self) -> (Self::Left, Self::Right);
}

/// Interface to read some structured data asynchronously
#[async_trait]
pub trait TypedAsyncRead<T> {
    /// Reads some data, returning `Some(T)` if available or `None` if the reader
    /// has closed and no longer is providing data
    async fn recv(&mut self) -> io::Result<Option<T>>;
}

/// Interface to write some structured data asynchronously
#[async_trait]
pub trait TypedAsyncWrite<T> {
    async fn send(&mut self, data: T) -> io::Result<()>;
}

mod router;

mod framed;
pub use framed::*;

mod inmemory;
pub use inmemory::*;

mod mpsc;
pub use mpsc::*;

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
