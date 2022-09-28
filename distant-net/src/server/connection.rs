use super::{ServerState, ShutdownTimer};
use crate::{
    auth::Verifier, ConnectionCtx, FramedTransport, Interest, Response, ServerCtx, ServerHandler,
    ServerReply, Transport, UntypedRequest,
};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    sync::{mpsc, RwLock},
    task::JoinHandle,
};

/// Time to wait inbetween connection read/write when nothing was read or written on last pass
const SLEEP_DURATION: Duration = Duration::from_millis(50);

/// Id associated with an active connection
pub type ConnectionId = u64;

/// Represents an individual connection on the server
pub struct Connection {
    /// Unique identifier tied to the connection
    id: ConnectionId,

    /// Task that is processing requests and responses
    task: JoinHandle<()>,
}

impl Connection {
    /// Starts building a new connection
    pub fn build() -> ConnectionBuilder<(), ()> {
        let id: ConnectionId = rand::random();
        ConnectionBuilder {
            id,
            handler: Weak::new(),
            state: Weak::new(),
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

    /// Aborts the connection
    pub fn abort(&self) {
        self.task.abort();
    }
}

pub struct ConnectionBuilder<H, T> {
    id: ConnectionId,
    handler: Weak<H>,
    state: Weak<ServerState>,
    transport: T,
    shutdown_timer: Weak<RwLock<ShutdownTimer>>,
    sleep_duration: Duration,
    verifier: Weak<Verifier>,
}

impl<H, T> ConnectionBuilder<H, T> {
    pub fn handler<U>(self, handler: Weak<U>) -> ConnectionBuilder<U, T> {
        ConnectionBuilder {
            id: self.id,
            handler,
            state: self.state,
            transport: self.transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier: self.verifier,
        }
    }

    pub fn state(self, state: Weak<ServerState>) -> ConnectionBuilder<H, T> {
        ConnectionBuilder {
            id: self.id,
            handler: self.handler,
            state,
            transport: self.transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier: self.verifier,
        }
    }

    pub fn transport<U>(self, transport: U) -> ConnectionBuilder<H, U> {
        ConnectionBuilder {
            id: self.id,
            handler: self.handler,
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
    ) -> ConnectionBuilder<H, T> {
        ConnectionBuilder {
            id: self.id,
            handler: self.handler,
            state: self.state,
            transport: self.transport,
            shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier: self.verifier,
        }
    }

    pub fn sleep_duration(self, sleep_duration: Duration) -> ConnectionBuilder<H, T> {
        ConnectionBuilder {
            id: self.id,
            handler: self.handler,
            state: self.state,
            transport: self.transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration,
            verifier: self.verifier,
        }
    }

    pub fn verifier(self, verifier: Weak<Verifier>) -> ConnectionBuilder<H, T> {
        ConnectionBuilder {
            id: self.id,
            handler: self.handler,
            state: self.state,
            transport: self.transport,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            verifier,
        }
    }
}

impl<H, T> ConnectionBuilder<H, T>
where
    H: ServerHandler + Sync + 'static,
    H::Request: DeserializeOwned + Send + Sync + 'static,
    H::Response: Serialize + Send + 'static,
    H::LocalData: Default + Send + Sync + 'static,
    T: Transport + Send + Sync + 'static,
{
    pub fn spawn(self) -> Connection {
        let id = self.id;

        Connection {
            id,
            task: tokio::spawn(self.run()),
        }
    }

    async fn run(self) {
        let ConnectionBuilder {
            id,
            handler,
            state,
            transport,
            shutdown_timer,
            sleep_duration,
            verifier,
        } = self;

        // Attempt to upgrade our handler for use with the connection going forward
        let handler = match Weak::upgrade(&handler) {
            Some(handler) => handler,
            None => {
                error!("[Conn {id}] Handler has been dropped");
                return;
            }
        };

        // Construct a queue of outgoing responses
        let (tx, mut rx) = mpsc::channel::<Response<H::Response>>(1);

        // Perform a handshake to ensure that the connection is properly established
        let mut transport: FramedTransport<T> = FramedTransport::plain(transport);
        if let Err(x) = transport.server_handshake().await {
            error!("[Conn {id}] Handshake failed: {x}");
            return;
        }

        // Perform authentication to ensure the connection is valid
        match Weak::upgrade(&verifier) {
            Some(verifier) => {
                if let Err(x) = verifier.verify(&mut transport).await {
                    error!("[Conn {id}] Verification failed: {x}");
                    return;
                }
            }
            None => {
                error!("[Conn {id}] Verifier has been dropped");
                return;
            }
        };

        // Create local data for the connection and then process it as well as perform
        // authentication and any other tasks on first connecting
        let mut local_data = H::LocalData::default();
        if let Err(x) = handler
            .on_accept(ConnectionCtx {
                connection_id: id,
                authenticator: &mut transport,
                local_data: &mut local_data,
            })
            .await
        {
            error!("[Conn {id}] Accepting connection failed: {x}");
            return;
        }

        let local_data = Arc::new(local_data);

        loop {
            let ready = transport
                .ready(Interest::READABLE | Interest::WRITABLE)
                .await
                .expect("[Conn {connection_id}] Failed to examine ready state");

            // Keep track of whether we read or wrote anything
            let mut read_blocked = false;
            let mut write_blocked = false;

            if ready.is_readable() {
                match transport.try_read_frame() {
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

                                handler.on_request(ctx).await;
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
                            error!("[Conn {id}] Invalid request: {x}");
                        }
                    },
                    Ok(None) => {
                        debug!("[Conn {id}] Connection closed");

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
                        break;
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
                        Ok(data) => match transport.try_write_frame(data) {
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
                    match transport.try_flush() {
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
