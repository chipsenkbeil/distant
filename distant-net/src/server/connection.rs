use super::{ConnectionCtx, ServerCtx, ServerHandler, ServerReply, ServerState, ShutdownTimer};
use crate::common::{
    authentication::{Keychain, Verifier},
    Backup, Connection, ConnectionId, Interest, Response, Transport, UntypedRequest,
};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot, RwLock},
    task::JoinHandle,
};

/// Time to wait inbetween connection read/write when nothing was read or written on last pass
const SLEEP_DURATION: Duration = Duration::from_nanos(1);

/// Represents an individual connection on the server
pub struct ConnectionTask {
    /// Unique identifier tied to the connection
    id: ConnectionId,

    /// Task that is processing requests and responses
    task: JoinHandle<()>,
}

impl ConnectionTask {
    /// Starts building a new connection
    pub fn build() -> ConnectionTaskBuilder<(), ()> {
        let id: ConnectionId = rand::random();
        ConnectionTaskBuilder {
            id,
            handler: Weak::new(),
            state: Weak::new(),
            keychain: Keychain::new(),
            transport: (),
            shutdown_timer: Weak::new(),
            sleep_duration: SLEEP_DURATION,
            verifier: Weak::new(),
        }
    }

    /// Returns the id associated with the connection
    pub fn id(&self) -> ConnectionId {
        self.id
    }

    /// Returns true if the task has finished
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Aborts the connection
    pub fn abort(&self) {
        self.task.abort();
    }
}

pub struct ConnectionTaskBuilder<H, T> {
    id: ConnectionId,
    handler: Weak<H>,
    state: Weak<ServerState>,
    keychain: Keychain<oneshot::Receiver<Backup>>,
    transport: T,
    shutdown_timer: Weak<RwLock<ShutdownTimer>>,
    sleep_duration: Duration,
    verifier: Weak<Verifier>,
}

impl<H, T> ConnectionTaskBuilder<H, T> {
    pub fn handler<U>(self, handler: Weak<U>) -> ConnectionTaskBuilder<U, T> {
        ConnectionTaskBuilder {
            id: self.id,
            handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier: self.verifier,
        }
    }

    pub fn state(self, state: Weak<ServerState>) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            id: self.id,
            handler: self.handler,
            state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier: self.verifier,
        }
    }

    pub fn keychain(
        self,
        keychain: Keychain<oneshot::Receiver<Backup>>,
    ) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            id: self.id,
            handler: self.handler,
            state: self.state,
            keychain,
            transport: self.transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier: self.verifier,
        }
    }

    pub fn transport<U>(self, transport: U) -> ConnectionTaskBuilder<H, U> {
        ConnectionTaskBuilder {
            id: self.id,
            handler: self.handler,
            keychain: self.keychain,
            state: self.state,
            transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier: self.verifier,
        }
    }

    pub(crate) fn shutdown_timer(
        self,
        shutdown_timer: Weak<RwLock<ShutdownTimer>>,
    ) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            id: self.id,
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier: self.verifier,
        }
    }

    pub fn sleep_duration(self, sleep_duration: Duration) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            id: self.id,
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration,
            verifier: self.verifier,
        }
    }

    pub fn verifier(self, verifier: Weak<Verifier>) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            id: self.id,
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier,
        }
    }
}

impl<H, T> ConnectionTaskBuilder<H, T>
where
    H: ServerHandler + Sync + 'static,
    H::Request: DeserializeOwned + Send + Sync + 'static,
    H::Response: Serialize + Send + 'static,
    H::LocalData: Default + Send + Sync + 'static,
    T: Transport + 'static,
{
    pub fn spawn(self) -> ConnectionTask {
        let id = self.id;

        ConnectionTask {
            id,
            task: tokio::spawn(self.run()),
        }
    }

    async fn run(self) {
        let ConnectionTaskBuilder {
            id,
            handler,
            state,
            keychain,
            transport,
            shutdown_timer,
            sleep_duration,
            verifier,
        } = self;

        // Will check if no more connections and restart timer if that's the case
        macro_rules! terminate_connection {
            // Prints an error message before terminating the connection
            (@error $($msg:tt)+) => {
                error!($($msg)+);
                terminate_connection!();
            };

            // Prints a debug message before terminating the connection
            (@debug $($msg:tt)+) => {
                debug!($($msg)+);
                terminate_connection!();
            };

            // Performs the connection termination by removing it from server state and
            // restarting the shutdown timer if it was the last connection
            () => {
                // Remove the connection from our state if it has closed
                if let Some(state) = Weak::upgrade(&state) {
                    state.connections.write().await.remove(&self.id);

                    // If we have no more connections, start the timer
                    if let Some(timer) = Weak::upgrade(&shutdown_timer) {
                        if state.connections.read().await.is_empty() {
                            timer.write().await.restart();
                        }
                    }
                }
                return;
            };
        }

        // Properly establish the connection's transport
        let mut connection = match Weak::upgrade(&verifier) {
            Some(verifier) => {
                match Connection::server(transport, verifier.as_ref(), keychain).await {
                    Ok(connection) => connection,
                    Err(x) => {
                        terminate_connection!(@error "[Conn {id}] Failed to setup connection: {x}");
                    }
                }
            }
            None => {
                terminate_connection!(@error "[Conn {id}] Verifier has been dropped");
            }
        };

        // Attempt to upgrade our handler for use with the connection going forward
        let handler = match Weak::upgrade(&handler) {
            Some(handler) => handler,
            None => {
                terminate_connection!(@error "[Conn {id}] Handler has been dropped");
            }
        };

        // Construct a queue of outgoing responses
        let (tx, mut rx) = mpsc::channel::<Response<H::Response>>(1);

        // Create local data for the connection and then process it
        debug!("[Conn {id}] Accepting connection");
        let mut local_data = H::LocalData::default();
        if let Err(x) = handler
            .on_accept(ConnectionCtx {
                connection_id: id,
                local_data: &mut local_data,
            })
            .await
        {
            terminate_connection!(@error "[Conn {id}] Accepting connection failed: {x}");
        }

        let local_data = Arc::new(local_data);

        loop {
            let ready = match connection
                .ready(Interest::READABLE | Interest::WRITABLE)
                .await
            {
                Ok(ready) => ready,
                Err(x) => {
                    terminate_connection!(@error "[Conn {id}] Failed to examine ready state: {x}");
                }
            };

            // Keep track of whether we read or wrote anything
            let mut read_blocked = !ready.is_readable();
            let mut write_blocked = !ready.is_writable();

            if ready.is_readable() {
                match connection.try_read_frame() {
                    Ok(Some(frame)) => match UntypedRequest::from_slice(frame.as_item()) {
                        Ok(request) => match request.to_typed_request() {
                            Ok(request) => {
                                let reply = ServerReply {
                                    origin_id: request.id.clone(),
                                    tx: tx.clone(),
                                };

                                let ctx = ServerCtx {
                                    connection_id: id,
                                    request,
                                    reply: reply.clone(),
                                    local_data: Arc::clone(&local_data),
                                };

                                // Spawn a new task to run the request handler so we don't block
                                // our connection from processing other requests
                                let handler = Arc::clone(&handler);
                                tokio::spawn(async move { handler.on_request(ctx).await });
                            }
                            Err(x) => {
                                if log::log_enabled!(Level::Trace) {
                                    trace!(
                                        "[Conn {id}] Failed receiving {}",
                                        String::from_utf8_lossy(&request.payload),
                                    );
                                }

                                error!("[Conn {id}] Invalid request: {x}");
                            }
                        },
                        Err(x) => {
                            error!("[Conn {id}] Invalid request payload: {x}");
                        }
                    },
                    Ok(None) => {
                        terminate_connection!(@debug "[Conn {id}] Connection closed");
                    }
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => read_blocked = true,
                    Err(x) => {
                        // NOTE: We do NOT break out of the loop, as this could happen
                        //       if someone sends bad data at any point, but does not
                        //       mean that the reader itself has failed. This can
                        //       happen from getting non-compliant typed data
                        error!("[Conn {id}] {x}");
                    }
                }
            }

            // If our socket is ready to be written to, we try to get the next item from
            // the queue and process it
            if ready.is_writable() {
                // If we get more data to write, attempt to write it, which will result in writing
                // any queued bytes as well. Othewise, we attempt to flush any pending outgoing
                // bytes that weren't sent earlier.
                if let Ok(response) = rx.try_recv() {
                    // Log our message as a string, which can be expensive
                    if log_enabled!(Level::Trace) {
                        trace!(
                            "[Conn {id}] Sending {}",
                            &response
                                .to_vec()
                                .map(|x| String::from_utf8_lossy(&x).to_string())
                                .unwrap_or_else(|_| "<Cannot serialize>".to_string())
                        );
                    }

                    match response.to_vec() {
                        Ok(data) => match connection.try_write_frame(data) {
                            Ok(()) => (),
                            Err(x) if x.kind() == io::ErrorKind::WouldBlock => write_blocked = true,
                            Err(x) => error!("[Conn {id}] Send failed: {x}"),
                        },
                        Err(x) => {
                            error!("[Conn {id}] Unable to serialize outgoing response: {x}");
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
                            error!("[Conn {id}] Failed to flush outgoing data: {x}");
                        }
                    }
                }
            }

            // If we did not read or write anything, sleep a bit to offload CPU usage
            if read_blocked && write_blocked {
                tokio::time::sleep(sleep_duration).await;
            }
        }
    }
}
