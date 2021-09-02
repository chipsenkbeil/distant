use super::{DataStream, SecretKey, Transport};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::{
    io,
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

pub type AcceptFuture<'a, T> = Pin<Box<dyn Future<Output = io::Result<T>> + Send + 'a>>;

/// Represents a type that has a listen interface for receiving raw streams
pub trait Listener: Send + Sync {
    type Output;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a;
}

//
// TODO: CHIP CHIP CHIP --
//
// Create a wrapper type instead of a trait directly on TcpStream and UnixStream
//
// 1. If accept() finishes, a new task is spawned to perform the handshake and
//    the join handle is added to a queue
// 2. On each loop, if the queue is not empty, a futures::future::select_all is run
//    alongside a tokio::select! with accept() to see if a new connection is received
//    or one of the existing handshakes finishes
//
//    https://docs.rs/futures/0.3.17/futures/future/fn.select_all.html
//
// Implement From<TcpStream> and From<UnixStream> ??? If we do, then the parent accept()
// would still need to be passed a context. That might be fine. We can also make it simple
// where an owned context is passed to avoid lifetime challenges

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListenerCtx {
    pub auth_key: Option<Arc<SecretKey>>,
    pub timeout: Duration,
}

pub struct TransportListener<T, L>
where
    T: DataStream,
    L: Listener<Output = T>,
{
    inner: L,
    queue: Vec<JoinHandle<io::Result<Transport<T>>>>,
}

impl<T, L> TransportListener<T, L>
where
    T: DataStream,
    L: Listener<Conn = T>,
{
    pub fn new(inner: L) -> Self {}

    pub async fn accept(&self) -> io::Result<Transport<T>> {}
}

async fn do_handshake<T>(
    stream: T,
    auth_key: Option<Arc<SecretKey>>,
    timeout: Duration,
) -> io::Result<Transport<T>>
where
    T: DataStream,
{
    tokio::select! {
        res = Transport::from_handshake(stream, auth_key) => {
            res
        }
        _ = tokio::time::sleep(timeout) => {
            Err(io::Error::from(io::ErrorKind::TimedOut))
        }
    }
}

impl Listener for TcpListener {
    type Output = TcpStream;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a,
    {
        async fn accept<'a>(_self: &'a TcpListener) -> io::Result<TcpStream> {
            _self.accept().await.map(|(stream, _)| stream)
        }

        Box::pin(accept(self))
    }
}

#[cfg(unix)]
impl Listener for tokio::net::UnixListener {
    type Output = tokio::net::UnixStream;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a,
    {
        async fn accept(_self: &tokio::net::UnixListener) -> io::Result<tokio::net::UnixStream> {
            _self.accept().await.map(|(stream, _)| stream)
        }

        Box::pin(accept(self))
    }
}

#[cfg(test)]
impl<T> Listener for tokio::sync::Mutex<tokio::sync::mpsc::Receiver<T>>
where
    T: DataStream + Send + Sync + 'static,
{
    type Output = T;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a,
    {
        async fn accept<'a, T>(
            _self: &'a tokio::sync::Mutex<tokio::sync::mpsc::Receiver<T>>,
        ) -> io::Result<T>
        where
            T: DataStream + Send + Sync + 'static,
        {
            _self
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
        }

        Box::pin(accept(self))
    }
}

#[cfg(test)]
impl<T> Listener for tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Transport<T>>>
where
    T: DataStream + Send + Sync + 'static,
{
    type Output = T;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a,
    {
        async fn accept<'a, T>(
            _self: &'a tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Transport<T>>>,
        ) -> io::Result<Transport<T>>
        where
            T: DataStream + Send + Sync + 'static,
        {
            _self.lock().await
            let res = tokio::select! {
                res = lock.recv() => {
                    res.ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
                }
                _ = tokio::time::sleep(timeout) => {
                    Err(io::Error::from(io::ErrorKind::TimedOut))
                }
            };

            let res = res?;

            Ok(Box::pin(async move { Ok(res) }))
        }

        Box::pin(accept(self))
    }
}
