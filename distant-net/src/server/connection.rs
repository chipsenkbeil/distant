use super::{ConnectionCtx, ServerCtx, ServerHandler, ServerReply, ServerState, ShutdownTimer};
use crate::common::{
    authentication::{Keychain, Verifier},
    Backup, Connection, ConnectionId, Frame, Interest, Response, Transport, UntypedRequest,
};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    future::Future,
    io,
    pin::Pin,
    sync::{Arc, Weak},
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    sync::{broadcast, mpsc, oneshot, RwLock},
    task::JoinHandle,
};

pub type ServerKeychain = Keychain<oneshot::Receiver<Backup>>;

/// Time to wait inbetween connection read/write when nothing was read or written on last pass.
const SLEEP_DURATION: Duration = Duration::from_millis(1);

/// Minimum time between heartbeats to communicate to the client connection.
const MINIMUM_HEARTBEAT_DURATION: Duration = Duration::from_secs(5);

/// Represents an individual connection on the server. If dropped, will terminate the connection.
pub struct ConnectionTask {
    /// Unique identifier tied to the connection.
    id: ConnectionId,

    /// Used to shutdown the connection as part of a global shutdown effort.
    global_shutdown: broadcast::Receiver<()>,

    /// Used to shutdown the connection.
    local_shutdown: Option<oneshot::Sender<()>>,

    /// Task that is processing requests and responses.
    task: JoinHandle<io::Result<()>>,
}

impl ConnectionTask {
    /// Starts building a new connection.
    pub fn build() -> ConnectionTaskBuilder<(), ()> {
        ConnectionTaskBuilder {
            handler: Weak::new(),
            state: Weak::new(),
            keychain: Keychain::new(),
            transport: (),
            global_shutdown: broadcast::channel(1).1,
            shutdown_timer: Weak::new(),
            sleep_duration: SLEEP_DURATION,
            heartbeat_duration: MINIMUM_HEARTBEAT_DURATION,
            verifier: Weak::new(),
        }
    }

    /// Returns the id associated with the connection.
    pub fn id(&self) -> ConnectionId {
        self.id
    }

    /// Returns true if the task has finished.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Sends a shutdown request to close the connection. Users should await on the connection to
    /// ensure that it has finished shutting down.
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.local_shutdown.take() {
            let _ = tx.send(());
        }
    }
}

impl Future for ConnectionTask {
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Future::poll(Pin::new(&mut self.task), cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(x) => match x {
                Ok(x) => Poll::Ready(x),
                Err(x) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, x))),
            },
        }
    }
}

pub struct ConnectionTaskBuilder<H, T> {
    handler: Weak<H>,
    state: Weak<ServerState>,
    keychain: Keychain<oneshot::Receiver<Backup>>,
    transport: T,
    global_shutdown: broadcast::Receiver<()>,
    shutdown_timer: Weak<RwLock<ShutdownTimer>>,
    sleep_duration: Duration,
    heartbeat_duration: Duration,
    verifier: Weak<Verifier>,
}

impl<H, T> ConnectionTaskBuilder<H, T> {
    pub fn handler<U>(self, handler: Weak<U>) -> ConnectionTaskBuilder<U, T> {
        ConnectionTaskBuilder {
            handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            global_shutdown: self.global_shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn state(self, state: Weak<ServerState>) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state,
            keychain: self.keychain,
            transport: self.transport,
            global_shutdown: self.global_shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn keychain(self, keychain: ServerKeychain) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain,
            transport: self.transport,
            global_shutdown: self.global_shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn transport<U>(self, transport: U) -> ConnectionTaskBuilder<H, U> {
        ConnectionTaskBuilder {
            handler: self.handler,
            keychain: self.keychain,
            state: self.state,
            transport,
            global_shutdown: self.global_shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn global_shutdown(
        self,
        global_shutdown: broadcast::Receiver<()>,
    ) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            global_shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn shutdown_timer(
        self,
        shutdown_timer: Weak<RwLock<ShutdownTimer>>,
    ) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            global_shutdown: self.global_shutdown,
            shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn sleep_duration(self, sleep_duration: Duration) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            global_shutdown: self.global_shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn heartbeat_duration(self, heartbeat_duration: Duration) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            global_shutdown: self.global_shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration,
            verifier: self.verifier,
        }
    }

    pub fn verifier(self, verifier: Weak<Verifier>) -> ConnectionTaskBuilder<H, T> {
        ConnectionTaskBuilder {
            handler: self.handler,
            state: self.state,
            keychain: self.keychain,
            transport: self.transport,
            global_shutdown: self.global_shutdown,
            shutdown_timer: self.shutdown_timer,
            sleep_duration: self.sleep_duration,
            heartbeat_duration: self.heartbeat_duration,
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
        let (tx, rx) = oneshot::channel();

        ConnectionTask {
            id,
            global_shutdown: self.global_shutdown,
            local_shutdown: Some(tx),
            task: tokio::spawn(self.run(rx)),
        }
    }

    async fn run(self, mut shutdown: oneshot::Receiver<()>) -> io::Result<()> {
        let ConnectionTaskBuilder {
            handler,
            state,
            keychain,
            transport,
            global_shutdown,
            shutdown_timer,
            sleep_duration,
            heartbeat_duration,
            verifier,
        } = self;

        // Construct a queue of outgoing responses
        let (tx, mut rx) = mpsc::channel::<Response<H::Response>>(1);

        // Will check if no more connections and restart timer if that's the case
        macro_rules! terminate_connection {
            // Prints an error message and stores state before terminating
            (@error $($msg:tt)+) => {
                error!($($msg)+);
                terminate_connection!();
                return Err(io::Error::new(io::ErrorKind::Other, $($msg)+));
            };

            // Prints a debug message and stores state before terminating
            (@debug $($msg:tt)+) => {
                debug!($($msg)+);
                terminate_connection!();
                return Ok(());
            };

            // Prints a shutdown message and stores state before terminating
            (@shutdown) => {
                debug!("[Conn {id}] Shutdown triggered");
                terminate_connection!();
                return Ok(());
            };

            // Performs the connection termination by removing it from server state and
            // restarting the shutdown timer if it was the last connection
            () => {
                // Remove the connection from our state if it has closed
                if let Some(state) = Weak::upgrade(&state) {
                    trace!("[Conn {id}] Placing self in closed connection list");
                    state.closed_connections.write().await.insert(self.id, );

                    trace!("[Conn {id}] Cleaning up self from active connection list");
                    state.connections.write().await.remove(&self.id);

                    // If we have no more connections, start the timer
                    if let Some(timer) = Weak::upgrade(&shutdown_timer) {
                        if state.connections.read().await.is_empty() {
                            trace!("[Conn {id}] Last connection terminating, so restarting shutdown timer");
                            timer.write().await.restart();
                        }
                    }
                }
            };
        }

        macro_rules! await_or_shutdown {
            ($future:expr) => {{
                tokio::select! {
                    _ = global_shutdown.recv() => {
                        terminate_connection!(@shutdown);
                    }
                    _ = &mut shutdown => {
                        terminate_connection!(@shutdown);
                    }
                    x = $future => { x }
                }
            }};
        }

        // Properly establish the connection's transport
        debug!("Establishing full connection using {transport:?}");
        let mut connection = match Weak::upgrade(&verifier) {
            Some(verifier) => {
                match await_or_shutdown!(Connection::server(transport, verifier.as_ref(), keychain))
                {
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

        // Update our id to be the connection id
        let id = connection.id();

        // Attempt to upgrade our handler for use with the connection going forward
        debug!("[Conn {id}] Preparing connection handler");
        let handler = match Weak::upgrade(&handler) {
            Some(handler) => handler,
            None => {
                terminate_connection!(@error "[Conn {id}] Handler has been dropped");
            }
        };

        // Create local data for the connection and then process it
        debug!("[Conn {id}] Officially accepting connection");
        let mut local_data = H::LocalData::default();
        if let Err(x) = await_or_shutdown!(handler.on_accept(ConnectionCtx {
            connection_id: id,
            local_data: &mut local_data
        })) {
            terminate_connection!(@error "[Conn {id}] Accepting connection failed: {x}");
        }

        let local_data = Arc::new(local_data);
        let mut last_heartbeat = Instant::now();

        // Store our connection details
        todo!();

        debug!("[Conn {id}] Beginning read/write loop");
        loop {
            let ready = match await_or_shutdown!(
                connection.ready(Interest::READABLE | Interest::WRITABLE)
            ) {
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
                                let ctx = ServerCtx {
                                    connection_id: id,
                                    request,
                                    reply: ServerReply {
                                        origin_id: request.id.clone(),
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
                        terminate_connection!(@debug "[Conn {id}] Connection closed");
                    }
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => read_blocked = true,
                    Err(x) => {
                        terminate_connection!(@error "[Conn {id}] {x}");
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
    use super::*;
    use crate::common::authentication::DummyAuthHandler;
    use crate::common::{
        HeapSecretKey, InmemoryTransport, Ready, Reconnectable, Request, Response,
    };
    use crate::server::Shutdown;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use test_log::test;

    struct TestServerHandler;

    #[async_trait]
    impl ServerHandler for TestServerHandler {
        type Request = u16;
        type Response = String;
        type LocalData = ();

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
            err.to_string().contains("Handler has been dropped"),
            "Unexpected error: {err}"
        );
    }

    #[test(tokio::test)]
    async fn should_terminate_if_accepting_connection_fails_on_server_handler() {
        struct BadAcceptServerHandler;

        #[async_trait]
        impl ServerHandler for BadAcceptServerHandler {
            type Request = u16;
            type Response = String;
            type LocalData = ();

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

        // NOTE: Termination is still an Ok(...) because we want to return some stateful info.
        //       This just verifies that we don't hang!
        task.await.unwrap();
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

        let mut conn = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .heartbeat_duration(Duration::from_millis(200))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Shutdown server connection task while it is establishing a full connection with the
        // client and verify that we get an error because we have not yet reached the point where
        // we would return an appropriate channel
        conn.shutdown();
        conn.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn should_be_able_to_shutdown_while_accepting_connection() {
        struct HangingAcceptServerHandler;

        #[async_trait]
        impl ServerHandler for HangingAcceptServerHandler {
            type Request = ();
            type Response = ();
            type LocalData = ();

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

        let mut conn = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .heartbeat_duration(Duration::from_millis(200))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle the client-side establishment of a full connection
        let _client_task = tokio::spawn(Connection::client(t2, DummyAuthHandler));

        // Shutdown server connection task while it is accepting the connection  and verify that we
        // get an error because we have not yet reached the point where we would return an
        // appropriate channel
        conn.shutdown();
        conn.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn should_be_able_to_shutdown_while_waiting_for_connection_to_be_ready() {
        struct AcceptServerHandler {
            tx: mpsc::Sender<()>,
        }

        #[async_trait]
        impl ServerHandler for AcceptServerHandler {
            type Request = ();
            type Response = ();
            type LocalData = ();

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

        let mut conn = ConnectionTask::build()
            .handler(Arc::downgrade(&handler))
            .state(Arc::downgrade(&state))
            .keychain(keychain)
            .transport(t1)
            .shutdown_timer(Arc::downgrade(&shutdown_timer))
            .heartbeat_duration(Duration::from_millis(200))
            .verifier(Arc::downgrade(&verifier))
            .spawn();

        // Spawn a task to handle the client-side establishment of a full connection
        let _client_task = tokio::spawn(Connection::client(t2, DummyAuthHandler));

        // Wait to ensure we complete the accept call first
        let _ = rx.recv().await;

        // Shutdown server connection task while it is accepting the connection  and verify that we
        // get an error because we have not yet reached the point where we would return an
        // appropriate channel
        conn.shutdown();
        conn.await.unwrap_err();
    }
}
