use super::{Codec, DataStream, Transport};
use futures::stream::Stream;
use log::*;
use std::{future::Future, pin::Pin};
use tokio::{
    io,
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinHandle,
};

/// Represents a [`Stream`] consisting of newly-connected [`DataStream`] instances that
/// have been wrapped in [`Transport`]
pub struct TransportListener<T, U>
where
    T: DataStream,
    U: Codec,
{
    listen_task: JoinHandle<()>,
    accept_task: JoinHandle<()>,
    rx: mpsc::Receiver<Transport<T, U>>,
}

impl<T, U> TransportListener<T, U>
where
    T: DataStream + Send + 'static,
    U: Codec + Send + 'static,
{
    pub fn initialize<L, F>(listener: L, mut make_transport: F) -> Self
    where
        L: Listener<Output = T> + 'static,
        F: FnMut(T) -> Transport<T, U> + Send + 'static,
    {
        let (stream_tx, mut stream_rx) = mpsc::channel::<T>(1);
        let listen_task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(stream) => {
                        if stream_tx.send(stream).await.is_err() {
                            error!("Listener failed to pass along stream");
                            break;
                        }
                    }
                    Err(x) => {
                        error!("Listener failed to accept stream: {}", x);
                        break;
                    }
                }
            }
        });

        let (tx, rx) = mpsc::channel::<Transport<T, U>>(1);
        let accept_task = tokio::spawn(async move {
            // Check if we have a new connection. If so, wrap it in a transport and forward
            // it along to
            while let Some(stream) = stream_rx.recv().await {
                let transport = make_transport(stream);
                if let Err(x) = tx.send(transport).await {
                    error!("Failed to forward transport: {}", x);
                }
            }
        });

        Self {
            listen_task,
            accept_task,
            rx,
        }
    }

    pub fn abort(&self) {
        self.listen_task.abort();
        self.accept_task.abort();
    }

    /// Waits for the next fully-initialized transport for an incoming stream to be available,
    /// returning none if no longer accepting new connections
    pub async fn accept(&mut self) -> Option<Transport<T, U>> {
        self.rx.recv().await
    }

    /// Converts into a stream of transport-wrapped connections
    pub fn into_stream(self) -> impl Stream<Item = Transport<T, U>> {
        futures::stream::unfold(self, |mut _self| async move {
            _self
                .accept()
                .await
                .map(move |transport| (transport, _self))
        })
    }
}

pub type AcceptFuture<'a, T> = Pin<Box<dyn Future<Output = io::Result<T>> + Send + 'a>>;

/// Represents a type that has a listen interface for receiving raw streams
pub trait Listener: Send + Sync {
    type Output;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a;
}

impl Listener for TcpListener {
    type Output = TcpStream;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
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
        async fn accept<T>(
            _self: &tokio::sync::Mutex<tokio::sync::mpsc::Receiver<T>>,
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
