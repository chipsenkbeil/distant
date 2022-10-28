use crate::common::{Connection, Interest, Reconnectable, Request, Transport, UntypedResponse};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io,
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::mpsc,
    task::{JoinError, JoinHandle},
};

mod builder;
pub use builder::*;

mod channel;
pub use channel::*;

mod reconnect;
pub use reconnect::*;

/// Time to wait inbetween connection read/write when nothing was read or written on last pass
const SLEEP_DURATION: Duration = Duration::from_millis(50);

/// Represents a client that can be used to send requests & receive responses from a server.
pub struct Client<T, U> {
    /// Used to send requests to a server
    channel: Channel<T, U>,

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
    /// Spawns a client using the provided [`Connection`].
    fn spawn<V>(mut connection: Connection<V>, mut strategy: ReconnectStrategy) -> Self
    where
        V: Transport + Send + Sync + 'static,
    {
        let post_office = Arc::new(PostOffice::default());
        let weak_post_office = Arc::downgrade(&post_office);
        let (tx, mut rx) = mpsc::channel::<Request<T>>(1);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

        // Ensure that our transport starts off clean (nothing in buffers or backup)
        connection.clear();

        // Start a task that continually checks for responses and delivers them using the
        // post office
        let task = tokio::spawn(async move {
            let mut needs_reconnect = false;

            loop {
                // If we have flagged that a reconnect is needed, attempt to do so
                if needs_reconnect {
                    info!("Client encountered issue, attempting to reconnect");
                    if log::log_enabled!(log::Level::Debug) {
                        debug!("Using strategy {strategy:?}");
                    }
                    match strategy.reconnect(&mut connection).await {
                        Ok(x) => {
                            needs_reconnect = false;
                            x
                        }
                        Err(x) => {
                            error!("Unable to re-establish connection: {x}");
                            break;
                        }
                    }
                }

                let ready = tokio::select! {
                    _ = shutdown_rx.recv() => {
                        debug!("Client got shutdown signal, so exiting event loop");
                        break;
                    }
                    result = connection.ready(Interest::READABLE | Interest::WRITABLE) => {
                        match result {
                            Ok(result) => result,
                            Err(x) => {
                                error!("Failed to examine ready state: {x}");
                                needs_reconnect = true;
                                continue;
                            }
                        }
                    }
                };

                let mut read_blocked = !ready.is_readable();
                let mut write_blocked = !ready.is_writable();

                if ready.is_readable() {
                    match connection.try_read_frame() {
                        Ok(Some(frame)) => {
                            match UntypedResponse::from_slice(frame.as_item()) {
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
                            }
                        }
                        Ok(None) => {
                            debug!("Connection closed");
                            needs_reconnect = true;
                            continue;
                        }
                        Err(x) if x.kind() == io::ErrorKind::WouldBlock => read_blocked = true,
                        Err(x) => {
                            error!("Failed to read next frame: {x}");
                        }
                    }
                }

                if ready.is_writable() {
                    // If we get more data to write, attempt to write it, which will result in
                    // writing any queued bytes as well. Othewise, we attempt to flush any pending
                    // outgoing bytes that weren't sent earlier.
                    if let Ok(request) = rx.try_recv() {
                        match request.to_vec() {
                            Ok(data) => match connection.try_write_frame(data) {
                                Ok(()) => (),
                                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                                    write_blocked = true
                                }
                                Err(x) => {
                                    error!("Send failed: {x}");
                                    needs_reconnect = true;
                                    continue;
                                }
                            },
                            Err(x) => {
                                error!("Unable to serialize outgoing request: {x}");
                            }
                        }
                    } else {
                        // In the case of flushing, there are two scenarios in which we want to
                        // mark no write occurring:
                        //
                        // 1. When flush did not write any bytes, which can happen when the buffer
                        //    is empty
                        // 2. When the call to write bytes blocks
                        match connection.try_flush() {
                            Ok(0) => write_blocked = true,
                            Ok(_) => (),
                            Err(x) if x.kind() == io::ErrorKind::WouldBlock => write_blocked = true,
                            Err(x) => {
                                error!("Failed to flush outgoing data: {x}");
                                needs_reconnect = true;
                                continue;
                            }
                        }
                    }
                }

                // If we did not read or write anything, sleep a bit to offload CPU usage
                if read_blocked && write_blocked {
                    tokio::time::sleep(SLEEP_DURATION).await;
                }
            }
        });

        let channel = Channel {
            tx,
            post_office: weak_post_office,
        };

        Self {
            channel,
            shutdown_tx,
            task,
        }
    }
}

impl Client<(), ()> {
    /// Creates a new [`ClientBuilder`]
    pub fn build() -> ClientBuilder<(), ()> {
        ClientBuilder::new()
    }

    /// Creates a new [`TcpClientBuilder`]
    pub fn tcp() -> TcpClientBuilder<()> {
        TcpClientBuilder::new()
    }

    /// Creates a new [`UnixSocketClientBuilder`]
    #[cfg(unix)]
    pub fn unix_socket() -> UnixSocketClientBuilder<()> {
        UnixSocketClientBuilder::new()
    }

    /// Creates a new [`WindowsPipeClientBuilder`]
    #[cfg(windows)]
    pub fn windows_pipe() -> WindowsPipeClientBuilder<()> {
        WindowsPipeClientBuilder::new()
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

    /// Waits for the client to terminate, which resolves when the receiving end of the network
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
