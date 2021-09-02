use super::{DataStream, SecretKey, Transport};
use futures::stream::Stream;
use log::*;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::{
    io,
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinHandle,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportListenerCtx {
    pub auth_key: Option<Arc<SecretKey>>,
    pub timeout: Duration,
}

/// Represents a [`Stream`] consisting of newly-connected [`DataStream`] instances that
/// have been wrapped in [`Transport`]
pub struct TransportListener<T>
where
    T: DataStream,
{
    listen_task: JoinHandle<()>,
    accept_task: JoinHandle<()>,
    rx: mpsc::Receiver<Transport<T>>,
}

impl<T> TransportListener<T>
where
    T: DataStream + Send + 'static,
{
    pub fn initialize<L>(listener: L, ctx: TransportListenerCtx) -> Self
    where
        L: Listener<Output = T> + 'static,
    {
        let (stream_tx, mut stream_rx) = mpsc::channel::<T>(1);
        let listen_task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(stream) => {
                        if stream_tx.send(stream).await.is_err() {
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

        let TransportListenerCtx { auth_key, timeout } = ctx;
        let (tx, rx) = mpsc::channel::<Transport<T>>(1);
        let accept_task = tokio::spawn(async move {
            let mut queue: Vec<JoinHandle<Transport<T>>> = Vec::new();

            // 1. If queue is empty, wait for a stream_rx input and add it to the queue
            // 2. If queue is not empty, perform a select between a select on the queue
            //    and stream_rx; if the queue returns a transport that is not an error,
            //    then we send it using tx
            loop {
                // If queue is empty, we wait for a stream to come in and queue up the handshake
                if queue.is_empty() {
                    match stream_rx.recv().await {
                        Some(stream) => {
                            let auth_key = auth_key.as_ref().cloned();
                            queue.push(tokio::spawn(async move {
                                do_handshake(stream, auth_key, timeout).await.unwrap()
                            }));
                        }
                        None => break,
                    }

                // Otherwise, we want to select across our queue and a new connection
                } else {
                    tokio::select! {
                        output = futures::future::select_all(queue.drain(..)) => {
                            let (res, _, remaining) = output;
                            queue.extend(remaining);
                            match res {
                                Ok(transport) => {
                                    if let Err(x) = tx.send(transport).await {
                                        error!("Failed to pass along transport: {}", x);
                                    }
                                }
                                Err(x) => {
                                    error!("Failed to stand up transport: {}", x);
                                }
                            }
                        }
                        res = stream_rx.recv() => {
                            match res {
                                Some(stream) => {
                                    let auth_key = auth_key.as_ref().cloned();
                                    queue.push(tokio::spawn(async move {
                                        do_handshake(stream, auth_key, timeout)
                                            .await
                                            .unwrap()
                                    }));
                                }
                                None => break,
                            }
                        }
                    }
                }
            }

            // Clean up our queue by aborting all remaining tasks
            for task in queue {
                task.abort();
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
    pub async fn accept(&mut self) -> Option<Transport<T>> {
        self.rx.recv().await
    }

    /// Converts into a stream of transport-wrapped connections
    pub fn into_stream(self) -> impl Stream<Item = Transport<T>> {
        futures::stream::unfold(self, |mut _self| async move {
            _self
                .accept()
                .await
                .map(move |transport| (transport, _self))
        })
    }
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
