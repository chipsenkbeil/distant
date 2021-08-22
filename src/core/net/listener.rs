use super::DataStream;
use std::{future::Future, pin::Pin};
use tokio::{
    io,
    net::{TcpListener, TcpStream},
};

/// Represents a type that has a listen interface
pub trait Listener: Send + Sync {
    type Conn: DataStream;

    /// Async function that accepts a new connection, returning `Ok(Self::Conn)`
    /// upon receiving the next connection
    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output = io::Result<Self::Conn>> + Send + 'a>>
    where
        Self: Sync + 'a;
}

impl Listener for TcpListener {
    type Conn = TcpStream;

    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output = io::Result<Self::Conn>> + Send + 'a>>
    where
        Self: Sync + 'a,
    {
        async fn accept(_self: &TcpListener) -> io::Result<TcpStream> {
            _self.accept().await.map(|(stream, _)| stream)
        }

        Box::pin(accept(self))
    }
}

#[cfg(unix)]
impl Listener for tokio::net::UnixListener {
    type Conn = tokio::net::UnixStream;

    fn accept<'a>(&'a self) -> Pin<Box<dyn Future<Output = io::Result<Self::Conn>> + Send + 'a>>
    where
        Self: Sync + 'a,
    {
        async fn accept(_self: &tokio::net::UnixListener) -> io::Result<tokio::net::UnixStream> {
            _self.accept().await.map(|(stream, _)| stream)
        }

        Box::pin(accept(self))
    }
}
