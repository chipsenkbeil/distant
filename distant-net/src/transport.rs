use async_trait::async_trait;
use std::{io, marker::Unpin};
use tokio::io::{AsyncRead, AsyncWrite};

/// Interface representing a transport of raw bytes into and out of the system
pub trait RawTransport:
    RawTransportRead + RawTransportWrite + IntoSplit<Write = Self::WriteHalf, Read = Self::ReadHalf>
{
    type ReadHalf: RawTransportRead;
    type WriteHalf: RawTransportWrite;
}

/// Interface representing a transport of raw bytes into the system
pub trait RawTransportRead: AsyncRead + Send + Unpin {}

/// Interface representing a transport of raw bytes out of the system
pub trait RawTransportWrite: AsyncWrite + Send + Unpin {}

/// Interface to split something into writing and reading halves
pub trait IntoSplit {
    type Write;
    type Read;

    fn into_split(self) -> (Self::Write, Self::Read);
}

/// Interface to read some structured data asynchronously
#[async_trait]
pub trait TypedAsyncRead<T> {
    /// Reads some data, returning `Some(T)` if available or `None` if the reader
    /// has closed and no longer is providing data
    async fn read(&mut self) -> io::Result<Option<T>>;
}

/// Interface to write some structured data asynchronously
#[async_trait]
pub trait TypedAsyncWrite<T> {
    async fn write(&mut self, data: T) -> io::Result<()>;
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
