use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use distant_auth::Verifier;
use log::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;

use super::{
    ConnectionCtx, ConnectionState, ServerCtx, ServerHandler, ServerReply, ServerState,
    ShutdownTimer,
};
use crate::common::{
    Backup, Connection, Frame, Interest, Keychain, Response, Transport, UntypedRequest,
};

pub type ServerKeychain = Keychain<oneshot::Receiver<Backup>>;

/// Time to wait inbetween connection read/write when nothing was read or written on last pass.
const SLEEP_DURATION: Duration = Duration::from_millis(1);

/// Minimum time between heartbeats to communicate to the client connection.
const MINIMUM_HEARTBEAT_DURATION: Duration = Duration::from_secs(5);

/// Represents an individual connection on the server.
pub(super) struct ConnectionTask(JoinHandle<io::Result<()>>);

impl ConnectionTask {
    /// Starts building a new connection
    pub fn build() -> ConnectionTaskBuilder<(), (), ()> {
        ConnectionTaskBuilder::new()
    }

    /// Returns true if the task has finished
    pub fn is_finished(&self) -> bool {
        self.0.is_finished()
    }
}

impl Future for ConnectionTask {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Future::poll(Pin::new(&mut self.0), cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(x) => match x {
                Ok(x) => Poll::Ready(x),
                Err(x) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, x))),
            },
        }
    }
}

/// Represents a builder for a new connection task.
pub(super) struct ConnectionTaskBuilder<H, S, T> {
    handler: Weak<H>,
    state: Weak<ServerState<S>>,
    keychain: Keychain<oneshot::Receiver<Backup>>,
    transport: T,
    shutdown: broadcast::Receiver<()>,
    shutdown_timer: Weak<RwLock<ShutdownTimer>>,
    sleep_duration: Duration,
    heartbeat_duration: Duration,
    verifier: Weak<Verifier>,
}

impl ConnectionTaskBuilder<(), (), ()> {
    /// Starts building a new connection.
    pub fn new() -> Self {
        Self {
            handler: Weak::new(),
            state: Weak::new(),
            keychain: Keychain::new(),
            transport: (),
            shutdown: broadcast::channel(1).1,
            shutdown_timer: Weak::new(),
            sleep_duration: SLEEP_DURATION,
            heartbeat_duration: MINIMUM_HEARTBEAT_DURATION,
            verifier: Weak::new(),
        }
    }
}

impl<H, S, T> ConnectionTaskBuilder<H, S, T> {
    pub fn handler<U>(self, handler: Weak<U>) -> ConnectionTaskBuilder<U, S, T> {
        ConnectionTaskBuilder {
            handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown: self.shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn state<U>(self, state: Weak<ServerState<U>>) -> ConnectionTaskBuilder<H, U, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown: self.shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn keychain(self, keychain: ServerKeychain) -> ConnectionTaskBuilder<H, S, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain,
            transport: self.transport,
            shutdown: self.shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn transport<U>(self, transport: U) -> ConnectionTaskBuilder<H, S, U> {
        ConnectionTaskBuilder {
            handler: self.handler,
            keychain: self.keychain,
            state: self.state,
            transport,
            shutdown: self.shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn shutdown(self, shutdown: broadcast::Receiver<()>) -> ConnectionTaskBuilder<H, S, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn shutdown_timer(
        self,
        shutdown_timer: Weak<RwLock<ShutdownTimer>>,
    ) -> ConnectionTaskBuilder<H, S, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown: self.shutdown,
            shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn sleep_duration(self, sleep_duration: Duration) -> ConnectionTaskBuilder<H, S, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown: self.shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn heartbeat_duration(
        self,
        heartbeat_duration: Duration,
    ) -> ConnectionTaskBuilder<H, S, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown: self.shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn verifier(self, verifier: Weak<Verifier>) -> ConnectionTaskBuilder<H, S, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            shutdown: self.shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier,
        }
    }
}

impl<H, T> ConnectionTaskBuilder<H, Response<H::Response>, T>
where
    H: ServerHandler + Sync + 'static,
    H::Request: DeserializeOwned + Send + Sync + 'static,
    H::Response: Serialize + Send + 'static,
    H::LocalData: Default + Send + Sync + 'static,
    T: Transport + 'static,
{
    pub fn spawn(self) -> ConnectionTask {
        ConnectionTask(tokio::spawn(self.run()))
    }

    async fn run(self) -> io::Result<()> {
        let ConnectionTaskBuilder {
            handler,
            state,
            keychain,
            transport,
            mut shutdown,
            shutdown_timer,
            sleep_duration,
            heartbeat_duration,
            verifier,
        } = self;

        // NOTE: This exists purely to make the compiler happy for macro_rules declaration order.
        let (mut local_shutdown, channel_tx, connection_state) = ConnectionState::channel();

        // Will check if no more connections and restart timer if that's the case
        macro_rules! terminate_connection {
            // Prints an error message and does not store state
            (@fatal $($msg:tt)+) => {
                error!($($msg)+);
                terminate_connection!();
                return Err(io::Error::new(io::ErrorKind::Other, format!($($msg)+)));
            };

            // Prints an error message and stores state before terminating
            (@error($tx:ident, $rx:ident) $($msg:tt)+) => {
                error!($($msg)+);
                terminate_connection!($tx, $rx);
                return Err(io::Error::new(io::ErrorKind::Other, format!($($msg)+)));
            };

            // Prints a debug message and stores state before terminating
            (@debug($tx:ident, $rx:ident) $($msg:tt)+) => {
                debug!($($msg)+);
                terminate_connection!($tx, $rx);
                return Ok(());
            };

            // Prints a shutdown message with no connection id and exit without sending state
            (@shutdown) => {
                debug!("Shutdown triggered before a connection could be fully established");
                terminate_connection!();
                return Ok(());
            };

            // Prints a shutdown message with no connection id and stores state before terminating
            (@shutdown) => {
                debug!("Shutdown triggered before a connection could be fully established");
                terminate_connection!();
                return Ok(());
            };

            // Prints a shutdown message and stores state before terminating
            (@shutdown($id:ident, $tx:ident, $rx:ident)) => {{
                debug!("[Conn {}] Shutdown triggered", $id);
                terminate_connection!($tx, $rx);
                return Ok(());
            }};

            // Performs the connection termination by removing it from server state and
            // restarting the shutdown timer if it was the last connection
            ($tx:ident, $rx:ident) => {
                // Send the channels back
                let _ = channel_tx.send(($tx, $rx));

                terminate_connection!();
            };

            // Performs the connection termination by removing it from server state and
            // restarting the shutdown timer if it was the last connection
            () => {
                // Restart our shutdown timer if this is the last connection
                if let Some(state) = Weak::upgrade(&state) {
                    if let Some(timer) = Weak::upgrade(&shutdown_timer) {
                        if state.connections.read().await.values().filter(|conn| !conn.is_finished()).count() <= 1 {
                            debug!("Last connection terminating, so restarting shutdown timer");
                            timer.write().await.restart();
                        }
                    }
                }
            };
        }

        /// Awaits a future to complete, or detects if a signal was received by either the global
        /// or local shutdown channel. Shutdown only occurs if a signal was received, and any
        /// errors received by either shutdown channel are ignored.
        macro_rules! await_or_shutdown {
            ($(@save($id:ident, $tx:ident, $rx:ident))? $future:expr) => {{
                let mut f = $future;

                loop {
                    let use_shutdown = match shutdown.try_recv() {
                        Ok(_) => {
                            terminate_connection!(@shutdown $(($id, $tx, $rx))?);
                        }
                        Err(broadcast::error::TryRecvError::Empty) => true,
                        Err(broadcast::error::TryRecvError::Lagged(_)) => true,
                        Err(broadcast::error::TryRecvError::Closed) => false,
                    };

                    let use_local_shutdown = match local_shutdown.try_recv() {
                        Ok(_) => {
                            terminate_connection!(@shutdown $(($id, $tx, $rx))?);
                        }
                        Err(oneshot::error::TryRecvError::Empty) => true,
                        Err(oneshot::error::TryRecvError::Closed) => false,
                    };

                    if use_shutdown && use_local_shutdown {
                        tokio::select! {
                            x = shutdown.recv() => {
                                if x.is_err() {
                                    continue;
                                }

                                terminate_connection!(@shutdown $(($id, $tx, $rx))?);
                            }
                            x = &mut local_shutdown => {
                                if x.is_err() {
                                    continue;
                                }

                                terminate_connection!(@shutdown $(($id, $tx, $rx))?);
                            }
                            x = &mut f => { break x; }
                        }
                    } else if use_shutdown {
                        tokio::select! {
                            x = shutdown.recv() => {
                                if x.is_err() {
                                    continue;
                                }

                                terminate_connection!(@shutdown $(($id, $tx, $rx))?);
                            }
                            x = &mut f => { break x; }
                        }
                    } else if use_local_shutdown {
                        tokio::select! {
                            x = &mut local_shutdown => {
                                if x.is_err() {
                                    continue;
                                }

                                terminate_connection!(@shutdown $(($id, $tx, $rx))?);
                            }
                            x = &mut f => { break x; }
                        }
                    } else {
                        break f.await;
                    }
                }
            }};
        }

        // Attempt to upgrade our handler for use with the connection going forward
        let handler = match Weak::upgrade(&handler) {
            Some(handler) => handler,
            None => {
                terminate_connection!(@fatal "Failed to setup connection because handler dropped");
            }
        };

        // Attempt to upgrade our state for use with the connection going forward
        let state = match Weak::upgrade(&state) {
            Some(state) => state,
            None => {
                terminate_connection!(@fatal "Failed to setup connection because state dropped");
            }
        };

        // Properly establish the connection's transport
        debug!("Establishing full connection using {transport:?}");
        let mut connection = match Weak::upgrade(&verifier) {
            Some(verifier) => {
                match await_or_shutdown!(Box::pin(Connection::server(
                    transport,
                    verifier.as_ref(),
                    keychain
                ))) {
                    Ok(connection) => connection,
                    Err(x) => {
                        terminate_connection!(@fatal "Failed to setup connection: {x}");
                    }
                }
            }
            None => {
                terminate_connection!(@fatal "Verifier has been dropped");
            }
        };

        // Update our id to be the connection id
        let id = connection.id();

        // Create local data for the connection and then process it
        debug!("[Conn {id}] Officially accepting connection");
        let mut local_data = H::LocalData::default();
        if let Err(x) = await_or_shutdown!(handler.on_accept(ConnectionCtx {
            connection_id: id,
            local_data: &mut local_data
        })) {
            terminate_connection!(@fatal "[Conn {id}] Accepting connection failed: {x}");
        }

        let local_data = Arc::new(local_data);
        let mut last_heartbeat = Instant::now();

        // Restore our connection's channels if we have them, otherwise make new ones
        let (tx, mut rx) = match state.connections.write().await.remove(&id) {
            Some(conn) => match conn.shutdown_and_wait().await {
                Some(x) => {
                    debug!("[Conn {id}] Marked as existing connection");
                    x
                }
                None => {
                    warn!("[Conn {id}] Existing connection with id, but channels not saved");
                    mpsc::channel::<Response<H::Response>>(1)
                }
            },
            None => {
                debug!("[Conn {id}] Marked as new connection");
                mpsc::channel::<Response<H::Response>>(1)
            }
        };

        // Store our connection details
        state.connections.write().await.insert(id, connection_state);

        debug!("[Conn {id}] Beginning read/write loop");
        loop {
            let ready = match await_or_shutdown!(
                @save(id, tx, rx)
                Box::pin(connection.ready(Interest::READABLE | Interest::WRITABLE))
            ) {
                Ok(ready) => ready,
                Err(x) => {
                    terminate_connection!(@error(tx, rx) "[Conn {id}] Failed to examine ready state: {x}");
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
                                let origin_id = request.id.clone();
                                let ctx = ServerCtx {
                                    connection_id: id,
                                    request,
                                    reply: ServerReply {
                                        origin_id,
                                        tx: tx.clone(),
                                    },
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
                        terminate_connection!(@debug(tx, rx) "[Conn {id}] Connection closed");
                    }
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => read_blocked = true,
                    Err(x) => {
                        terminate_connection!(@error(tx, rx) "[Conn {id}] {x}");
                    }
                }
            }

            // If our socket is ready to be written to, we try to get the next item from
            // the queue and process it
            if ready.is_writable() {
                // Send a heartbeat if we have exceeded our last time
                if last_heartbeat.elapsed() >= heartbeat_duration {
                    trace!("[Conn {id}] Sending heartbeat via empty frame");
                    match connection.try_write_frame(Frame::empty()) {
                        Ok(()) => (),
                        Err(x) if x.kind() == io::ErrorKind::WouldBlock => write_blocked = true,
                        Err(x) => error!("[Conn {id}] Send failed: {x}"),
                    }
                    last_heartbeat = Instant::now();
                }
                // If we get more data to write, attempt to write it, which will result in writing
                // any queued bytes as well. Othewise, we attempt to flush any pending outgoing
                // bytes that weren't sent earlier.
                else if let Ok(response) = rx.try_recv() {
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;
    use distant_auth::DummyAuthHandler;
    use test_log::test;

    use super::*;
    use crate::common::{
        HeapSecretKey, InmemoryTransport, Ready, Reconnectable, Request, Response,
    };
    use crate::server::Shutdown;

    struct TestServerHandler;

    #[async_trait]
    impl ServerHandler for TestServerHandler {
        type LocalData = ();
        type Request = u16;
        type Response = String;

        async fn on_accept(&self, _: ConnectionCtx<'_, Self::LocalData>) -> io::Result<()> {
            Ok(())
        }

        async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
            // Always send back "hello"
            ctx.reply.send("hello".to_string()).await.unwrap();
        }
    }

    macro_rules! wait_for_termination {
        ($task:ident) => {{
            let timeout_millis = 500;
            let sleep_millis = 50;
            let start = std::time::Instant::now();
            while !$task.is_finished() {
                if start.elapsed() > std::time::Duration::from_millis(timeout_millis) {
                    panic!("Exceeded timeout of {timeout_millis}ms");
                }
                tokio::time::sleep(std::time::Duration::from_millis(sleep_millis)).await;
            }
        }};
    }

    #[test(tokio::test)]
    async fn should_terminate_if_fails_access_verifier() {
        let handler = Arc::new(TestServerHandler);
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, _t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));

        let task = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .verifier(Weak::new())
            .spawn();

        wait_for_termination!(task);

        let err = task.await.unwrap_err();
        assert!(
            err.to_string().contains("Verifier has been dropped"),
            "Unexpected error: {err}"
        );
    }

    #[test(tokio::test)]
    async fn should_terminate_if_fails_to_setup_server_connection() {
        let handler = Arc::new(TestServerHandler);
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));

        // Create a verifier that wants a key, so we will fail from client-side
        let verifier = Arc::new(Verifier::static_key(HeapSecretKey::generate(32).unwrap()));

        let task = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle establishing connection from client-side
        tokio::spawn(async move {
            let _client = Connection::client(t2, DummyAuthHandler)
                .await
                .expect("Fail to establish client-side connection");
        });

        wait_for_termination!(task);

        let err = task.await.unwrap_err();
        assert!(
            err.to_string().contains("Failed to setup connection"),
            "Unexpected error: {err}"
        );
    }

    #[test(tokio::test)]
    async fn should_terminate_if_fails_access_server_handler() {
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));
        let verifier = Arc::new(Verifier::none());

        let task = ConnectionTask::build()
            .handler(Weak::<TestServerHandler>::new())
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle establishing connection from client-side
        tokio::spawn(async move {
            let _client = Connection::client(t2, DummyAuthHandler)
                .await
                .expect("Fail to establish client-side connection");
        });

        wait_for_termination!(task);

        let err = task.await.unwrap_err();
        assert!(
            err.to_string().contains("handler dropped"),
            "Unexpected error: {err}"
        );
    }

    #[test(tokio::test)]
    async fn should_terminate_if_accepting_connection_fails_on_server_handler() {
        struct BadAcceptServerHandler;

        #[async_trait]
        impl ServerHandler for BadAcceptServerHandler {
            type LocalData = ();
            type Request = u16;
            type Response = String;

            async fn on_accept(&self, _: ConnectionCtx<'_, Self::LocalData>) -> io::Result<()> {
                Err(io::Error::new(io::ErrorKind::Other, "bad accept"))
            }

            async fn on_request(
                &self,
                _: ServerCtx<Self::Request, Self::Response, Self::LocalData>,
            ) {
                unreachable!();
            }
        }

        let handler = Arc::new(BadAcceptServerHandler);
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));
        let verifier = Arc::new(Verifier::none());

        let task = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle establishing connection from client-side, and then closes to
        // trigger the server-side to close
        tokio::spawn(async move {
            let _client = Connection::client(t2, DummyAuthHandler)
                .await
                .expect("Fail to establish client-side connection");
        });

        wait_for_termination!(task);

        let err = task.await.unwrap_err();
        assert!(
            err.to_string().contains("Accepting connection failed"),
            "Unexpected error: {err}"
        );
    }

    #[test(tokio::test)]
    async fn should_terminate_if_connection_fails_to_become_ready() {
        let handler = Arc::new(TestServerHandler);
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));
        let verifier = Arc::new(Verifier::none());

        #[derive(Debug)]
        struct FakeTransport {
            inner: InmemoryTransport,
            fail_ready: Arc<AtomicBool>,
        }

        #[async_trait]
        impl Transport for FakeTransport {
            fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
                self.inner.try_read(buf)
            }

            fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
                self.inner.try_write(buf)
            }

            async fn ready(&self, interest: Interest) -> io::Result<Ready> {
                if self.fail_ready.load(Ordering::Relaxed) {
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        "targeted ready failure",
                    ))
                } else {
                    self.inner.ready(interest).await
                }
            }
        }

        #[async_trait]
        impl Reconnectable for FakeTransport {
            async fn reconnect(&mut self) -> io::Result<()> {
                self.inner.reconnect().await
            }
        }

        let fail_ready = Arc::new(AtomicBool::new(false));
        let task = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(FakeTransport {
                inner: t1,
                fail_ready: Arc::clone(&fail_ready),
            })
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle establishing connection from client-side, set ready to fail
        // for the server-side after client connection completes, and wait a bit
        tokio::spawn(async move {
            let _client = Connection::client(t2, DummyAuthHandler)
                .await
                .expect("Fail to establish client-side connection");

            // NOTE: Need to sleep for a little bit to hand control back to server to finish
            //       its side of the connection before toggling ready to fail
            tokio::time::sleep(Duration::from_millis(50)).await;

            // Toggle ready to fail and then wait awhile so we fail by ready and not connection
            // being dropped
            fail_ready.store(true, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_secs(1)).await;
        });

        wait_for_termination!(task);

        let err = task.await.unwrap_err();
        assert!(
            err.to_string().contains("targeted ready failure"),
            "Unexpected error: {err}"
        );
    }

    #[test(tokio::test)]
    async fn should_terminate_if_connection_closes() {
        let handler = Arc::new(TestServerHandler);
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));
        let verifier = Arc::new(Verifier::none());

        let task = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle establishing connection from client-side, and then closes to
        // trigger the server-side to close
        tokio::spawn(async move {
            let _client = Connection::client(t2, DummyAuthHandler)
                .await
                .expect("Fail to establish client-side connection");
        });

        wait_for_termination!(task);
        task.await.unwrap();
    }

    #[test(tokio::test)]
    async fn should_invoke_server_handler_to_process_request_in_new_task_and_forward_responses() {
        let handler = Arc::new(TestServerHandler);
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));
        let verifier = Arc::new(Verifier::none());

        let _conn = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle establishing connection from client-side
        let task = tokio::spawn(async move {
            let mut client = Connection::client(t2, DummyAuthHandler)
                .await
                .expect("Fail to establish client-side connection");

            client.write_frame_for(&Request::new(123u16)).await.unwrap();
            client
                .read_frame_as::<Response<String>>()
                .await
                .unwrap()
                .unwrap()
        });

        let response = task.await.unwrap();
        assert_eq!(response.payload, "hello");
    }

    #[test(tokio::test)]
    async fn should_send_heartbeat_via_empty_frame_every_minimum_duration() {
        let handler = Arc::new(TestServerHandler);
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));
        let verifier = Arc::new(Verifier::none());

        let _conn = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .heartbeat_duration(Duration::from_millis(200))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle establishing connection from client-side
        let task = tokio::spawn(async move {
            let mut client = Connection::client(t2, DummyAuthHandler)
                .await
                .expect("Fail to establish client-side connection");

            // Verify we don't get a frame immediately
            assert_eq!(
                client.try_read_frame().unwrap_err().kind(),
                io::ErrorKind::WouldBlock,
                "got a frame early"
            );

            // Sleep more than our minimum heartbeat duration to ensure we get one
            tokio::time::sleep(Duration::from_millis(250)).await;
            assert_eq!(
                client.read_frame().await.unwrap().unwrap(),
                Frame::empty(),
                "non-empty frame"
            );

            // Verify we don't get a frame immediately
            assert_eq!(
                client.try_read_frame().unwrap_err().kind(),
                io::ErrorKind::WouldBlock,
                "got a frame early"
            );

            // Sleep more than our minimum heartbeat duration to ensure we get one
            tokio::time::sleep(Duration::from_millis(250)).await;
            assert_eq!(
                client.read_frame().await.unwrap().unwrap(),
                Frame::empty(),
                "non-empty frame"
            );
        });

        task.await.unwrap();
    }

    #[test(tokio::test)]
    async fn should_be_able_to_shutdown_while_establishing_connection() {
        let handler = Arc::new(TestServerHandler);
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, _t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));
        let verifier = Arc::new(Verifier::none());

        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let conn = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown(shutdown_rx)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .heartbeat_duration(Duration::from_millis(200))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Shutdown server connection task while it is establishing a full connection with the
        // client, verifying that we do not get an error in return
        shutdown_tx
            .send(())
            .expect("Failed to send shutdown signal");
        conn.await.unwrap();
    }

    #[test(tokio::test)]
    async fn should_be_able_to_shutdown_while_accepting_connection() {
        struct HangingAcceptServerHandler;

        #[async_trait]
        impl ServerHandler for HangingAcceptServerHandler {
            type LocalData = ();
            type Request = ();
            type Response = ();

            async fn on_accept(&self, _: ConnectionCtx<'_, Self::LocalData>) -> io::Result<()> {
                // Wait "forever" so we can ensure that we fail at this step
                tokio::time::sleep(Duration::MAX).await;
                Err(io::Error::new(io::ErrorKind::Other, "bad accept"))
            }

            async fn on_request(
                &self,
                _: ServerCtx<Self::Request, Self::Response, Self::LocalData>,
            ) {
                unreachable!();
            }
        }

        let handler = Arc::new(HangingAcceptServerHandler);
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));
        let verifier = Arc::new(Verifier::none());

        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let conn = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown(shutdown_rx)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .heartbeat_duration(Duration::from_millis(200))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle the client-side establishment of a full connection
        let _client_task = tokio::spawn(Connection::client(t2, DummyAuthHandler));

        // Shutdown server connection task while it is accepting the connection, verifying that we
        // do not get an error in return
        shutdown_tx
            .send(())
            .expect("Failed to send shutdown signal");
        conn.await.unwrap();
    }

    #[test(tokio::test)]
    async fn should_be_able_to_shutdown_while_waiting_for_connection_to_be_ready() {
        struct AcceptServerHandler {
            tx: mpsc::Sender<()>,
        }

        #[async_trait]
        impl ServerHandler for AcceptServerHandler {
            type LocalData = ();
            type Request = ();
            type Response = ();

            async fn on_accept(&self, _: ConnectionCtx<'_, Self::LocalData>) -> io::Result<()> {
                self.tx.send(()).await.unwrap();
                Ok(())
            }

            async fn on_request(
                &self,
                _: ServerCtx<Self::Request, Self::Response, Self::LocalData>,
            ) {
                unreachable!();
            }
        }

        let (tx, mut rx) = mpsc::channel(100);
        let handler = Arc::new(AcceptServerHandler { tx });
        let state = Arc::new(ServerState::default());
        let keychain = ServerKeychain::new();
        let (t1, t2) = InmemoryTransport::pair(100);
        let shutdown_timer = Arc::new(RwLock::new(ShutdownTimer::start(Shutdown::Never)));
        let verifier = Arc::new(Verifier::none());

        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
        let conn = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown(shutdown_rx)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .heartbeat_duration(Duration::from_millis(200))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle the client-side establishment of a full connection
        let _client_task = tokio::spawn(Connection::client(t2, DummyAuthHandler));

        // Wait to ensure we complete the accept call first
        let _ = rx.recv().await;

        // Shutdown server connection task while it is accepting the connection, verifying that we
        // do not get an error in return
        shutdown_tx
            .send(())
            .expect("Failed to send shutdown signal");
        conn.await.unwrap();
    }
}
