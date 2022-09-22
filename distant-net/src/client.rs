use crate::{FramedTransport, Interest, Reconnectable, Request, Transport, UntypedResponse};
use async_trait::async_trait;
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};
use tokio::{
    io,
    sync::{mpsc, oneshot},
    task::{JoinError, JoinHandle},
};

mod channel;
pub use channel::*;

mod ext;
pub use ext::*;

/// Represents a client that can be used to send requests & receive responses from a server
pub struct Client<T, U> {
    /// Used to send requests to a server
    channel: Channel<T, U>,

    /// Used to send reconnect request to inner transport
    reconnect_tx: mpsc::Sender<oneshot::Sender<io::Result<()>>>,

    /// Used to send shutdown request to inner transport
    shutdown_tx: mpsc::Sender<()>,

    /// Contains the task that is running to send requests and receive responses from a server
    task: JoinHandle<()>,
}

impl<T, U> Client<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Creates a client using the provided [`FramedTransport`].
    ///
    /// ### Note
    ///
    /// It is assumed that the provided transport has performed any necessary handshake and is
    /// fully authenticated.
    pub fn new<V, const CAPACITY: usize>(mut transport: FramedTransport<V, CAPACITY>) -> Self
    where
        V: Transport + Send + Sync + 'static,
    {
        let post_office = Arc::new(PostOffice::default());
        let weak_post_office = Arc::downgrade(&post_office);
        let (tx, mut rx) = mpsc::channel::<Request<T>>(1);
        let (reconnect_tx, mut reconnect_rx) = mpsc::channel::<oneshot::Sender<io::Result<()>>>(1);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

        // Start a task that continually checks for responses and delivers them using the
        // post office
        let task = tokio::spawn(async move {
            loop {
                let ready = tokio::select! {
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                    cb = reconnect_rx.recv() => {
                        if let Some(cb) = cb {
                            let _ = cb.send(Reconnectable::reconnect(&mut transport).await);
                            continue;
                        } else {
                            break;
                        }
                    }
                    result = transport.ready(Interest::READABLE | Interest::WRITABLE) => {
                        result.expect("Failed to examine ready state")
                    }
                };

                if ready.is_readable() {
                    match transport.try_read_frame() {
                        Ok(Some(frame)) => match UntypedResponse::from_slice(frame.as_item()) {
                            Ok(response) => {
                                match response.to_typed_response() {
                                    Ok(response) => {
                                        // Try to send response to appropriate mailbox
                                        // TODO: This will block if full... is that a problem?
                                        // TODO: How should we handle false response? Did logging in past
                                        post_office.deliver_response(response).await;
                                    }
                                    Err(x) => {
                                        if log::log_enabled!(Level::Trace) {
                                            trace!(
                                                "Failed receiving {}",
                                                String::from_utf8_lossy(&response.payload),
                                            );
                                        }

                                        error!("Invalid response: {x}");
                                    }
                                }
                            }
                            Err(x) => {
                                error!("Invalid response: {x}");
                            }
                        },
                        Ok(None) => (),
                        Err(x) if x.kind() == io::ErrorKind::WouldBlock => (),
                        Err(x) => {
                            error!("Failed to read next frame: {x}");
                        }
                    }
                }

                if ready.is_writable() {
                    if let Ok(request) = rx.try_recv() {
                        match request.to_vec() {
                            Ok(data) => match transport.try_write_frame(data) {
                                Ok(()) => (),
                                Err(x) if x.kind() == io::ErrorKind::WouldBlock => (),
                                Err(x) => error!("Send failed: {x}"),
                            },
                            Err(x) => {
                                error!("Unable to serialize outgoing request: {x}");
                            }
                        }
                    }

                    match transport.try_flush() {
                        Ok(()) => (),
                        Err(x) if x.kind() == io::ErrorKind::WouldBlock => (),
                        Err(x) => {
                            error!("Failed to flush outgoing data: {x}");
                        }
                    }
                }
            }
        });

        let channel = Channel {
            tx,
            post_office: weak_post_office,
        };

        Self {
            channel,
            reconnect_tx,
            shutdown_tx,
            task,
        }
    }
}

impl<T, U> Client<T, U> {
    /// Convert into underlying channel
    pub fn into_channel(self) -> Channel<T, U> {
        self.channel
    }

    /// Clones the underlying channel for requests and returns the cloned instance
    pub fn clone_channel(&self) -> Channel<T, U> {
        self.channel.clone()
    }

    /// Waits for the client to terminate, which results when the receiving end of the network
    /// connection is closed (or the client is shutdown)
    pub async fn wait(self) -> Result<(), JoinError> {
        self.task.await
    }

    /// Abort the client's current connection by forcing its tasks to abort
    pub fn abort(&self) {
        self.task.abort();
    }

    /// Signal for the client to shutdown its connection cleanly
    pub async fn shutdown(&self) -> bool {
        self.shutdown_tx.send(()).await.is_ok()
    }

    /// Returns true if client's underlying event processing has finished/terminated
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

#[async_trait]
impl<T, U> Reconnectable for Client<T, U>
where
    T: Send,
    U: Send + Sync,
{
    async fn reconnect(&mut self) -> io::Result<()> {
        let (tx, rx) = oneshot::channel();
        if self.reconnect_tx.send(tx).await.is_ok() {
            rx.await
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "Callback lost"))?
        } else {
            Err(io::Error::new(
                io::ErrorKind::Other,
                "Client internal task dead",
            ))
        }
    }
}

impl<T, U> Deref for Client<T, U> {
    type Target = Channel<T, U>;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

impl<T, U> DerefMut for Client<T, U> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.channel
    }
}

impl<T, U> From<Client<T, U>> for Channel<T, U> {
    fn from(client: Client<T, U>) -> Self {
        client.channel
    }
}
