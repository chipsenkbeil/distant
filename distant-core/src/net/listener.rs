use std::{future::Future, pin::Pin};
use tokio::io;

mod transport;
pub use transport::TransportListener;

mod tcp;

#[cfg(test)]
mod test;

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

pub type AcceptFuture<'a, T> = Pin<Box<dyn Future<Output = io::Result<T>> + Send + 'a>>;

/// Represents a type that has a listen interface for receiving raw streams
pub trait Listener: Send + Sync {
    type Output;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a;
}
