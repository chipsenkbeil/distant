use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{fmt, io};

use log::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::common::{
    Connection, FramedTransport, HeapSecretKey, InmemoryTransport, Interest, Reconnectable,
    Transport, UntypedRequest, UntypedResponse,
};

mod builder;
pub use builder::*;

mod channel;
pub use channel::*;

mod config;
pub use config::*;

mod reconnect;
pub use reconnect::*;

mod shutdown;
pub use shutdown::*;

/// Time to wait inbetween connection read/write when nothing was read or written on last pass
const SLEEP_DURATION: Duration = Duration::from_millis(1);

/// Represents a client that can be used to send requests & receive responses from a server.
///
/// ### Note
///
/// This variant does not validate the payload of requests or responses being sent and received.
pub struct UntypedClient {
    /// Used to send requests to a server.
    channel: UntypedChannel,

    /// Used to watch for changes in the connection state.
    watcher: ConnectionWatcher,

    /// Used to send shutdown request to inner task.
    shutdown: Box<dyn Shutdown>,

    /// Indicates whether the client task will be shutdown when the client is dropped.
    shutdown_on_drop: bool,

    /// Contains the task that is running to send requests and receive responses from a server.
    task: Option<JoinHandle<io::Result<()>>>,
}

impl fmt::Debug for UntypedClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UntypedClient")
            .field("channel", &self.channel)
            .field("shutdown", &"...")
            .field("task", &self.task)
            .field("shutdown_on_drop", &self.shutdown_on_drop)
            .finish()
    }
}

impl Drop for UntypedClient {
    fn drop(&mut self) {
        if self.shutdown_on_drop {
            // TODO: Shutdown is an async operation, can we use it here?
            if let Some(task) = self.task.take() {
                debug!("Shutdown on drop = true, so aborting client task");
                task.abort();
            }
        }
    }
}

impl UntypedClient {
    /// Consumes the client, returning a typed variant.
    pub fn into_typed_client<T, U>(mut self) -> Client<T, U> {
        Client {
            channel: self.clone_channel().into_typed_channel(),
            watcher: self.watcher.clone(),
            shutdown: self.shutdown.clone(),
            shutdown_on_drop: self.shutdown_on_drop,
            task: self.task.take(),
        }
    }

    /// Convert into underlying channel.
    pub fn into_channel(self) -> UntypedChannel {
        self.clone_channel()
    }

    /// Clones the underlying channel for requests and returns the cloned instance.
    pub fn clone_channel(&self) -> UntypedChannel {
        self.channel.clone()
    }

    /// Waits for the client to terminate, which resolves when the receiving end of the network
    /// connection is closed (or the client is shutdown). Returns whether or not the client exited
    /// successfully or due to an error.
    pub async fn wait(mut self) -> io::Result<()> {
        match self.task.take().unwrap().await {
            Ok(x) => x,
            Err(x) => Err(io::Error::new(io::ErrorKind::Other, x)),
        }
    }

    /// Abort the client's current connection by forcing its tasks to abort.
    pub fn abort(&self) {
        if let Some(task) = self.task.as_ref() {
            task.abort();
        }
    }

    /// Clones the underlying shutdown signaler for the client. This enables you to wait on the
    /// client while still having the option to shut it down from somewhere else.
    pub fn clone_shutdown(&self) -> Box<dyn Shutdown> {
        self.shutdown.clone()
    }

    /// Signal for the client to shutdown its connection cleanly.
    pub async fn shutdown(&self) -> io::Result<()> {
        self.shutdown.shutdown().await
    }

    /// Returns whether the client should fully shutdown once it is dropped. If true, this will
    /// result in all channels tied to the client no longer functioning once the client is dropped.
    pub fn will_shutdown_on_drop(&mut self) -> bool {
        self.shutdown_on_drop
    }

    /// Sets whether the client should fully shutdown once it is dropped. If true, this will result
    /// in all channels tied to the client no longer functioning once the client is dropped.
    pub fn shutdown_on_drop(&mut self, shutdown_on_drop: bool) {
        self.shutdown_on_drop = shutdown_on_drop;
    }

    /// Clones the underlying [`ConnectionStateWatcher`] for the client.
    pub fn clone_connection_watcher(&self) -> ConnectionWatcher {
        self.watcher.clone()
    }

    /// Spawns a new task that continually monitors for connection changes and invokes the function
    /// `f` whenever a new change is detected.
    pub fn on_connection_change<F>(&self, f: F) -> JoinHandle<()>
    where
        F: FnMut(ConnectionState) + Send + 'static,
    {
        self.watcher.on_change(f)
    }

    /// Returns true if client's underlying event processing has finished/terminated.
    pub fn is_finished(&self) -> bool {
        self.task.is_none() || self.task.as_ref().unwrap().is_finished()
    }

    /// Spawns a client using the provided [`FramedTransport`] of [`InmemoryTransport`] and a
    /// specific [`ReconnectStrategy`].
    ///
    /// ### Note
    ///
    /// This will NOT perform any handshakes or authentication procedures nor will it replay any
    /// missing frames. This is to be used when establishing a [`Client`] to be run internally
    /// within a program.
    pub fn spawn_inmemory(
        transport: FramedTransport<InmemoryTransport>,
        config: ClientConfig,
    ) -> Self {
        let connection = Connection::Client {
            id: rand::random(),
            reauth_otp: HeapSecretKey::generate(32).unwrap(),
            transport,
        };
        Self::spawn(connection, config)
    }

    /// Spawns a client using the provided [`Connection`].
    pub(crate) fn spawn<V>(mut connection: Connection<V>, config: ClientConfig) -> Self
    where
        V: Transport + 'static,
    {
        let post_office = Arc::new(PostOffice::default());
        let weak_post_office = Arc::downgrade(&post_office);
        let (tx, mut rx) = mpsc::channel::<UntypedRequest<'static>>(1);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<oneshot::Sender<io::Result<()>>>(1);

        // Ensure that our transport starts off clean (nothing in buffers or backup)
        connection.clear();

        let ClientConfig {
            mut reconnect_strategy,
            shutdown_on_drop,
            silence_duration,
        } = config;

        // Start a task that continually checks for responses and delivers them using the
        // post office
        let shutdown_tx_2 = shutdown_tx.clone();
        let (watcher_tx, watcher_rx) = watch::channel(ConnectionState::Connected);
        let task = tokio::spawn(async move {
            let mut needs_reconnect = false;
            let mut last_read_frame_time = Instant::now();

            // NOTE: We hold onto a copy of the shutdown sender, even though we will never use it,
            //       to prevent the channel from being closed. This is because we do a check to
            //       see if we get a shutdown signal or ready state, and closing the channel
            //       would cause recv() to resolve immediately and result in the task shutting
            //       down.
            let _shutdown_tx = shutdown_tx_2;

            loop {
                // If we have flagged that a reconnect is needed, attempt to do so
                if needs_reconnect {
                    info!("Client encountered issue, attempting to reconnect");
                    if log::log_enabled!(log::Level::Debug) {
                        debug!("Using strategy {reconnect_strategy:?}");
                    }
                    match reconnect_strategy.reconnect(&mut connection).await {
                        Ok(()) => {
                            info!("Client successfully reconnected!");
                            needs_reconnect = false;
                            last_read_frame_time = Instant::now();
                            watcher_tx.send_replace(ConnectionState::Connected);
                        }
                        Err(x) => {
                            error!("Unable to re-establish connection: {x}");
                            watcher_tx.send_replace(ConnectionState::Disconnected);
                            break Err(x);
                        }
                    }
                }

                macro_rules! silence_needs_reconnect {
                    () => {{
                        debug!(
                            "Client exceeded {}s without server activity, so attempting to reconnect",
                            silence_duration.as_secs_f32(),
                        );
                        needs_reconnect = true;
                        watcher_tx.send_replace(ConnectionState::Reconnecting);
                        continue;
                    }};
                }

                let silence_time_remaining = silence_duration
                    .checked_sub(last_read_frame_time.elapsed())
                    .unwrap_or_default();

                // NOTE: sleep will not trigger if duration is zero/nanosecond scale, so we
                //       instead will do an early check here in the case that we need to reconnect
                //       prior to a sleep while polling the ready status
                if silence_time_remaining.as_millis() == 0 {
                    silence_needs_reconnect!();
                }

                let ready = tokio::select! {
                    // NOTE: This should NEVER return None as we never allow the channel to close.
                    cb = shutdown_rx.recv() => {
                        debug!("Client got shutdown signal, so exiting event loop");
                        let cb = cb.expect("Impossible: shutdown channel closed!");
                        let _ = cb.send(Ok(()));
                        watcher_tx.send_replace(ConnectionState::Disconnected);
                        break Ok(());
                    }
                    _ = tokio::time::sleep(silence_time_remaining) => {
                        silence_needs_reconnect!();
                    }
                    result = connection.ready(Interest::READABLE | Interest::WRITABLE) => {
                        match result {
                            Ok(result) => result,
                            Err(x) => {
                                error!("Failed to examine ready state: {x}");
                                needs_reconnect = true;
                                watcher_tx.send_replace(ConnectionState::Reconnecting);
                                continue;
                            }
                        }
                    }
                };

                let mut read_blocked = !ready.is_readable();
                let mut write_blocked = !ready.is_writable();

                if ready.is_readable() {
                    match connection.try_read_frame() {
                        // If we get an empty frame, we consider this a heartbeat and want
                        // to adjust our frame read time and discard it from our backup
                        Ok(Some(frame)) if frame.is_empty() => {
                            trace!("Client received heartbeat");
                            last_read_frame_time = Instant::now();
                        }

                        // Otherwise, we attempt to parse a frame as a response
                        Ok(Some(frame)) => {
                            last_read_frame_time = Instant::now();
                            match UntypedResponse::from_slice(frame.as_item()) {
                                Ok(response) => {
                                    if log_enabled!(Level::Trace) {
                                        trace!(
                                            "Client receiving (id:{} | origin: {}): {}",
                                            response.id,
                                            response.origin_id,
                                            String::from_utf8_lossy(&response.payload).to_string()
                                        );
                                    }

                                    // For trace-level logging, we need to clone the id and
                                    // origin id before passing the response ownership to
                                    // be delivered elsewhere
                                    let (id, origin_id) = if log_enabled!(Level::Trace) {
                                        (response.id.to_string(), response.origin_id.to_string())
                                    } else {
                                        (String::new(), String::new())
                                    };

                                    // Try to send response to appropriate mailbox
                                    // TODO: This will block if full... is that a problem?
                                    if post_office
                                        .deliver_untyped_response(response.into_owned())
                                        .await
                                    {
                                        trace!("Client delivered response {id} to {origin_id}");
                                    } else {
                                        trace!("Client dropped response {id} to {origin_id}");
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
                            watcher_tx.send_replace(ConnectionState::Reconnecting);
                            continue;
                        }
                        Err(x) if x.kind() == io::ErrorKind::WouldBlock => read_blocked = true,
                        Err(x) => {
                            error!("Failed to read next frame: {x}");
                            needs_reconnect = true;
                            watcher_tx.send_replace(ConnectionState::Reconnecting);
                            continue;
                        }
                    }
                }

                if ready.is_writable() {
                    // If we get more data to write, attempt to write it, which will result in
                    // writing any queued bytes as well. Othewise, we attempt to flush any pending
                    // outgoing bytes that weren't sent earlier.
                    if let Ok(request) = rx.try_recv() {
                        if log_enabled!(Level::Trace) {
                            trace!(
                                "Client sending {}",
                                String::from_utf8_lossy(&request.to_bytes()).to_string()
                            );
                        }
                        match connection.try_write_frame(request.to_bytes()) {
                            Ok(()) => (),
                            Err(x) if x.kind() == io::ErrorKind::WouldBlock => write_blocked = true,
                            Err(x) => {
                                error!("Failed to write frame: {x}");
                                needs_reconnect = true;
                                watcher_tx.send_replace(ConnectionState::Reconnecting);
                                continue;
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
                                watcher_tx.send_replace(ConnectionState::Reconnecting);
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

        let channel = UntypedChannel {
            tx,
            post_office: weak_post_office,
        };

        Self {
            channel,
            watcher: ConnectionWatcher(watcher_rx),
            shutdown: Box::new(shutdown_tx),
            shutdown_on_drop,
            task: Some(task),
        }
    }
}

impl Deref for UntypedClient {
    type Target = UntypedChannel;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

impl DerefMut for UntypedClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.channel
    }
}

impl From<UntypedClient> for UntypedChannel {
    fn from(client: UntypedClient) -> Self {
        client.into_channel()
    }
}

/// Represents a client that can be used to send requests & receive responses from a server.
pub struct Client<T, U> {
    /// Used to send requests to a server.
    channel: Channel<T, U>,

    /// Used to watch for changes in the connection state.
    watcher: ConnectionWatcher,

    /// Used to send shutdown request to inner task.
    shutdown: Box<dyn Shutdown>,

    /// Indicates whether the client task will be shutdown when the client is dropped.
    shutdown_on_drop: bool,

    /// Contains the task that is running to send requests and receive responses from a server.
    task: Option<JoinHandle<io::Result<()>>>,
}

impl<T, U> fmt::Debug for Client<T, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("channel", &self.channel)
            .field("shutdown", &"...")
            .field("task", &self.task)
            .field("shutdown_on_drop", &self.shutdown_on_drop)
            .finish()
    }
}

impl<T, U> Drop for Client<T, U> {
    fn drop(&mut self) {
        if self.shutdown_on_drop {
            // TODO: Shutdown is an async operation, can we use it here?
            if let Some(task) = self.task.take() {
                debug!("Shutdown on drop = true, so aborting client task");
                task.abort();
            }
        }
    }
}

impl<T, U> Client<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Consumes the client, returning an untyped variant.
    pub fn into_untyped_client(mut self) -> UntypedClient {
        UntypedClient {
            channel: self.clone_channel().into_untyped_channel(),
            watcher: self.watcher.clone(),
            shutdown: self.shutdown.clone(),
            shutdown_on_drop: self.shutdown_on_drop,
            task: self.task.take(),
        }
    }

    /// Spawns a client using the provided [`FramedTransport`] of [`InmemoryTransport`] and a
    /// specific [`ReconnectStrategy`].
    ///
    /// ### Note
    ///
    /// This will NOT perform any handshakes or authentication procedures nor will it replay any
    /// missing frames. This is to be used when establishing a [`Client`] to be run internally
    /// within a program.
    pub fn spawn_inmemory(
        transport: FramedTransport<InmemoryTransport>,
        config: ClientConfig,
    ) -> Self {
        UntypedClient::spawn_inmemory(transport, config).into_typed_client()
    }
}

impl Client<(), ()> {
    /// Creates a new [`ClientBuilder`].
    pub fn build() -> ClientBuilder<(), ()> {
        ClientBuilder::new()
    }

    /// Creates a new [`ClientBuilder`] configured to use a [`TcpConnector`].
    pub fn tcp<T>(connector: impl Into<TcpConnector<T>>) -> ClientBuilder<(), TcpConnector<T>> {
        ClientBuilder::new().connector(connector.into())
    }

    /// Creates a new [`ClientBuilder`] configured to use a [`UnixSocketConnector`].
    #[cfg(unix)]
    pub fn unix_socket(
        connector: impl Into<UnixSocketConnector>,
    ) -> ClientBuilder<(), UnixSocketConnector> {
        ClientBuilder::new().connector(connector.into())
    }

    /// Creates a new [`ClientBuilder`] configured to use a local [`WindowsPipeConnector`].
    #[cfg(windows)]
    pub fn local_windows_pipe(
        connector: impl Into<WindowsPipeConnector>,
    ) -> ClientBuilder<(), WindowsPipeConnector> {
        let mut connector = connector.into();
        connector.local = true;
        ClientBuilder::new().connector(connector)
    }

    /// Creates a new [`ClientBuilder`] configured to use a [`WindowsPipeConnector`].
    #[cfg(windows)]
    pub fn windows_pipe(
        connector: impl Into<WindowsPipeConnector>,
    ) -> ClientBuilder<(), WindowsPipeConnector> {
        ClientBuilder::new().connector(connector.into())
    }
}

impl<T, U> Client<T, U> {
    /// Convert into underlying channel.
    pub fn into_channel(self) -> Channel<T, U> {
        self.clone_channel()
    }

    /// Clones the underlying channel for requests and returns the cloned instance.
    pub fn clone_channel(&self) -> Channel<T, U> {
        self.channel.clone()
    }

    /// Waits for the client to terminate, which resolves when the receiving end of the network
    /// connection is closed (or the client is shutdown). Returns whether or not the client exited
    /// successfully or due to an error.
    pub async fn wait(mut self) -> io::Result<()> {
        match self.task.take().unwrap().await {
            Ok(x) => x,
            Err(x) => Err(io::Error::new(io::ErrorKind::Other, x)),
        }
    }

    /// Abort the client's current connection by forcing its tasks to abort.
    pub fn abort(&self) {
        if let Some(task) = self.task.as_ref() {
            task.abort();
        }
    }

    /// Clones the underlying shutdown signaler for the client. This enables you to wait on the
    /// client while still having the option to shut it down from somewhere else.
    pub fn clone_shutdown(&self) -> Box<dyn Shutdown> {
        self.shutdown.clone()
    }

    /// Signal for the client to shutdown its connection cleanly.
    pub async fn shutdown(&self) -> io::Result<()> {
        self.shutdown.shutdown().await
    }

    /// Returns whether the client should fully shutdown once it is dropped. If true, this will
    /// result in all channels tied to the client no longer functioning once the client is dropped.
    pub fn will_shutdown_on_drop(&mut self) -> bool {
        self.shutdown_on_drop
    }

    /// Sets whether the client should fully shutdown once it is dropped. If true, this will result
    /// in all channels tied to the client no longer functioning once the client is dropped.
    pub fn shutdown_on_drop(&mut self, shutdown_on_drop: bool) {
        self.shutdown_on_drop = shutdown_on_drop;
    }

    /// Clones the underlying [`ConnectionStateWatcher`] for the client.
    pub fn clone_connection_watcher(&self) -> ConnectionWatcher {
        self.watcher.clone()
    }

    /// Spawns a new task that continually monitors for connection changes and invokes the function
    /// `f` whenever a new change is detected.
    pub fn on_connection_change<F>(&self, f: F) -> JoinHandle<()>
    where
        F: FnMut(ConnectionState) + Send + 'static,
    {
        self.watcher.on_change(f)
    }

    /// Returns true if client's underlying event processing has finished/terminated.
    pub fn is_finished(&self) -> bool {
        self.task.is_none() || self.task.as_ref().unwrap().is_finished()
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
        client.clone_channel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientConfig;
    use crate::common::{Ready, Request, Response, TestTransport};

    mod typed {
        use test_log::test;

        use super::*;
        type TestClient = Client<u8, u8>;

        fn spawn_test_client<T>(
            connection: Connection<T>,
            reconnect_strategy: ReconnectStrategy,
        ) -> TestClient
        where
            T: Transport + 'static,
        {
            UntypedClient::spawn(
                connection,
                ClientConfig {
                    reconnect_strategy,
                    ..Default::default()
                },
            )
            .into_typed_client()
        }

        /// Creates a new test transport whose operations do not panic, but do nothing.
        #[inline]
        fn new_test_transport() -> TestTransport {
            TestTransport {
                f_try_read: Box::new(|_| Err(io::ErrorKind::WouldBlock.into())),
                f_try_write: Box::new(|_| Err(io::ErrorKind::WouldBlock.into())),
                f_ready: Box::new(|_| Ok(Ready::EMPTY)),
                f_reconnect: Box::new(|| Ok(())),
            }
        }

        #[test(tokio::test)]
        async fn should_write_queued_requests_as_outgoing_frames() {
            let (client, mut server) = Connection::pair(100);

            let mut client = spawn_test_client(client, ReconnectStrategy::Fail);
            client.fire(Request::new(1u8)).await.unwrap();
            client.fire(Request::new(2u8)).await.unwrap();
            client.fire(Request::new(3u8)).await.unwrap();

            assert_eq!(
                server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap()
                    .payload,
                1
            );
            assert_eq!(
                server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap()
                    .payload,
                2
            );
            assert_eq!(
                server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap()
                    .payload,
                3
            );
        }

        #[test(tokio::test)]
        async fn should_read_incoming_frames_as_responses_and_deliver_them_to_waiting_mailboxes() {
            let (client, mut server) = Connection::pair(100);

            // NOTE: Spawn a separate task to handle the response so we do not deadlock
            tokio::spawn(async move {
                let request = server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap();
                server
                    .write_frame_for(&Response::new(request.id, 2u8))
                    .await
                    .unwrap();
            });

            let mut client = spawn_test_client(client, ReconnectStrategy::Fail);
            assert_eq!(client.send(Request::new(1u8)).await.unwrap().payload, 2);
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_fails_to_determine_state() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            spawn_test_client(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    transport.f_ready = Box::new(|_| Err(io::ErrorKind::Other.into()));

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ReconnectStrategy::FixedInterval {
                    interval: Duration::from_millis(50),
                    max_retries: None,
                    timeout: None,
                },
            );

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_closed_by_server() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            spawn_test_client(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    // Report back that we're readable to trigger try_read
                    transport.f_ready = Box::new(|_| Ok(Ready::READABLE));

                    // Report that no bytes were written, indicting the channel was closed
                    transport.f_try_read = Box::new(|_| Ok(0));

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ReconnectStrategy::FixedInterval {
                    interval: Duration::from_millis(50),
                    max_retries: None,
                    timeout: None,
                },
            );

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_errors_while_reading_data() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            spawn_test_client(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    // Report back that we're readable to trigger try_read
                    transport.f_ready = Box::new(|_| Ok(Ready::READABLE));

                    // Fail the read
                    transport.f_try_read = Box::new(|_| Err(io::ErrorKind::Other.into()));

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ReconnectStrategy::FixedInterval {
                    interval: Duration::from_millis(50),
                    max_retries: None,
                    timeout: None,
                },
            );

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_unable_to_send_new_request() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            let mut client = spawn_test_client(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    // Report back that we're readable to trigger try_read
                    transport.f_ready = Box::new(|_| Ok(Ready::WRITABLE));

                    // Fail the write
                    transport.f_try_write = Box::new(|_| Err(io::ErrorKind::Other.into()));

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ReconnectStrategy::FixedInterval {
                    interval: Duration::from_millis(50),
                    max_retries: None,
                    timeout: None,
                },
            );

            // Queue up a request to fail to send
            client
                .fire(Request::new(123u8))
                .await
                .expect("Failed to queue request");

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_unable_to_flush_an_existing_request() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            let mut client = spawn_test_client(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    // Report back that we're readable to trigger try_read
                    transport.f_ready = Box::new(|_| Ok(Ready::WRITABLE));

                    // Succeed partially with initial try_write, block on second call, and then
                    // fail during a try_flush
                    transport.f_try_write = Box::new(|buf| unsafe {
                        static mut CNT: u8 = 0;
                        CNT += 1;
                        if CNT == 1 {
                            Ok(buf.len() / 2)
                        } else if CNT == 2 {
                            Err(io::ErrorKind::WouldBlock.into())
                        } else {
                            Err(io::ErrorKind::Other.into())
                        }
                    });

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ReconnectStrategy::FixedInterval {
                    interval: Duration::from_millis(50),
                    max_retries: None,
                    timeout: None,
                },
            );

            // Queue up a request to fail to send
            client
                .fire(Request::new(123u8))
                .await
                .expect("Failed to queue request");

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_exit_if_reconnect_strategy_has_failed_to_connect() {
            let (client, server) = Connection::pair(100);

            // Spawn the client, verify the task is running, kill our server, and verify that the
            // client does not block trying to reconnect
            let client = spawn_test_client(client, ReconnectStrategy::Fail);
            assert!(!client.is_finished(), "Client unexpectedly died");
            drop(server);
            assert_eq!(
                client.wait().await.unwrap_err().kind(),
                io::ErrorKind::ConnectionAborted
            );
        }

        #[test(tokio::test)]
        async fn should_exit_if_shutdown_signal_detected() {
            let (client, _server) = Connection::pair(100);

            let client = spawn_test_client(client, ReconnectStrategy::Fail);
            client.shutdown().await.unwrap();

            // NOTE: We wait for the client's task to conclude by using `wait` to ensure we do not
            //       have a race condition testing the task finished state. This will also verify
            //       that the task exited cleanly, rather than panicking.
            client.wait().await.unwrap();
        }

        #[test(tokio::test)]
        async fn should_not_exit_if_shutdown_channel_is_closed() {
            let (client, mut server) = Connection::pair(100);

            // NOTE: Spawn a separate task to handle the response so we do not deadlock
            tokio::spawn(async move {
                let request = server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap();
                server
                    .write_frame_for(&Response::new(request.id, 2u8))
                    .await
                    .unwrap();
            });

            // NOTE: We consume the client to produce a channel without maintaining the shutdown
            //       channel in order to ensure that dropping the client does not kill the task.
            let mut channel = spawn_test_client(client, ReconnectStrategy::Fail).into_channel();
            assert_eq!(channel.send(Request::new(1u8)).await.unwrap().payload, 2);
        }
    }

    mod untyped {
        use test_log::test;

        use super::*;
        type TestClient = UntypedClient;

        /// Creates a new test transport whose operations do not panic, but do nothing.
        #[inline]
        fn new_test_transport() -> TestTransport {
            TestTransport {
                f_try_read: Box::new(|_| Err(io::ErrorKind::WouldBlock.into())),
                f_try_write: Box::new(|_| Err(io::ErrorKind::WouldBlock.into())),
                f_ready: Box::new(|_| Ok(Ready::EMPTY)),
                f_reconnect: Box::new(|| Ok(())),
            }
        }

        #[test(tokio::test)]
        async fn should_write_queued_requests_as_outgoing_frames() {
            let (client, mut server) = Connection::pair(100);

            let mut client = TestClient::spawn(client, Default::default());
            client
                .fire(Request::new(1u8).to_untyped_request().unwrap())
                .await
                .unwrap();
            client
                .fire(Request::new(2u8).to_untyped_request().unwrap())
                .await
                .unwrap();
            client
                .fire(Request::new(3u8).to_untyped_request().unwrap())
                .await
                .unwrap();

            assert_eq!(
                server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap()
                    .payload,
                1
            );
            assert_eq!(
                server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap()
                    .payload,
                2
            );
            assert_eq!(
                server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap()
                    .payload,
                3
            );
        }

        #[test(tokio::test)]
        async fn should_read_incoming_frames_as_responses_and_deliver_them_to_waiting_mailboxes() {
            let (client, mut server) = Connection::pair(100);

            // NOTE: Spawn a separate task to handle the response so we do not deadlock
            tokio::spawn(async move {
                let request = server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap();
                server
                    .write_frame_for(&Response::new(request.id, 2u8))
                    .await
                    .unwrap();
            });

            let mut client = TestClient::spawn(client, Default::default());
            assert_eq!(
                client
                    .send(Request::new(1u8).to_untyped_request().unwrap())
                    .await
                    .unwrap()
                    .to_typed_response::<u8>()
                    .unwrap()
                    .payload,
                2
            );
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_fails_to_determine_state() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            TestClient::spawn(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    transport.f_ready = Box::new(|_| Err(io::ErrorKind::Other.into()));

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ClientConfig {
                    reconnect_strategy: ReconnectStrategy::FixedInterval {
                        interval: Duration::from_millis(50),
                        max_retries: None,
                        timeout: None,
                    },
                    ..Default::default()
                },
            );

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_closed_by_server() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            TestClient::spawn(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    // Report back that we're readable to trigger try_read
                    transport.f_ready = Box::new(|_| Ok(Ready::READABLE));

                    // Report that no bytes were written, indicting the channel was closed
                    transport.f_try_read = Box::new(|_| Ok(0));

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ClientConfig {
                    reconnect_strategy: ReconnectStrategy::FixedInterval {
                        interval: Duration::from_millis(50),
                        max_retries: None,
                        timeout: None,
                    },
                    ..Default::default()
                },
            );

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_errors_while_reading_data() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            TestClient::spawn(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    // Report back that we're readable to trigger try_read
                    transport.f_ready = Box::new(|_| Ok(Ready::READABLE));

                    // Fail the read
                    transport.f_try_read = Box::new(|_| Err(io::ErrorKind::Other.into()));

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ClientConfig {
                    reconnect_strategy: ReconnectStrategy::FixedInterval {
                        interval: Duration::from_millis(50),
                        max_retries: None,
                        timeout: None,
                    },
                    ..Default::default()
                },
            );

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_unable_to_send_new_request() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            let mut client = TestClient::spawn(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    // Report back that we're readable to trigger try_read
                    transport.f_ready = Box::new(|_| Ok(Ready::WRITABLE));

                    // Fail the write
                    transport.f_try_write = Box::new(|_| Err(io::ErrorKind::Other.into()));

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ClientConfig {
                    reconnect_strategy: ReconnectStrategy::FixedInterval {
                        interval: Duration::from_millis(50),
                        max_retries: None,
                        timeout: None,
                    },
                    ..Default::default()
                },
            );

            // Queue up a request to fail to send
            client
                .fire(Request::new(123u8).to_untyped_request().unwrap())
                .await
                .expect("Failed to queue request");

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_connection_unable_to_flush_an_existing_request() {
            let (reconnect_tx, mut reconnect_rx) = mpsc::channel(1);
            let mut client = TestClient::spawn(
                Connection::test_client({
                    let mut transport = new_test_transport();

                    // Report back that we're readable to trigger try_read
                    transport.f_ready = Box::new(|_| Ok(Ready::WRITABLE));

                    // Succeed partially with initial try_write, block on second call, and then
                    // fail during a try_flush
                    transport.f_try_write = Box::new(|buf| unsafe {
                        static mut CNT: u8 = 0;
                        CNT += 1;
                        if CNT == 1 {
                            Ok(buf.len() / 2)
                        } else if CNT == 2 {
                            Err(io::ErrorKind::WouldBlock.into())
                        } else {
                            Err(io::ErrorKind::Other.into())
                        }
                    });

                    // Send a signal that the reconnect happened while marking it successful
                    transport.f_reconnect = Box::new(move || {
                        reconnect_tx.try_send(()).expect("reconnect tx blocked");
                        Ok(())
                    });

                    transport
                }),
                ClientConfig {
                    reconnect_strategy: ReconnectStrategy::FixedInterval {
                        interval: Duration::from_millis(50),
                        max_retries: None,
                        timeout: None,
                    },
                    ..Default::default()
                },
            );

            // Queue up a request to fail to send
            client
                .fire(Request::new(123u8).to_untyped_request().unwrap())
                .await
                .expect("Failed to queue request");

            reconnect_rx.recv().await.expect("Reconnect did not occur");
        }

        #[test(tokio::test)]
        async fn should_exit_if_reconnect_strategy_has_failed_to_connect() {
            let (client, server) = Connection::pair(100);

            // Spawn the client, verify the task is running, kill our server, and verify that the
            // client does not block trying to reconnect
            let client = TestClient::spawn(client, Default::default());
            assert!(!client.is_finished(), "Client unexpectedly died");
            drop(server);
            assert_eq!(
                client.wait().await.unwrap_err().kind(),
                io::ErrorKind::ConnectionAborted
            );
        }

        #[test(tokio::test)]
        async fn should_exit_if_shutdown_signal_detected() {
            let (client, _server) = Connection::pair(100);

            let client = TestClient::spawn(client, Default::default());
            client.shutdown().await.unwrap();

            // NOTE: We wait for the client's task to conclude by using `wait` to ensure we do not
            //       have a race condition testing the task finished state. This will also verify
            //       that the task exited cleanly, rather than panicking.
            client.wait().await.unwrap();
        }

        #[test(tokio::test)]
        async fn should_not_exit_if_shutdown_channel_is_closed() {
            let (client, mut server) = Connection::pair(100);

            // NOTE: Spawn a separate task to handle the response so we do not deadlock
            tokio::spawn(async move {
                let request = server
                    .read_frame_as::<Request<u8>>()
                    .await
                    .unwrap()
                    .unwrap();
                server
                    .write_frame_for(&Response::new(request.id, 2u8))
                    .await
                    .unwrap();
            });

            // NOTE: We consume the client to produce a channel without maintaining the shutdown
            //       channel in order to ensure that dropping the client does not kill the task.
            let mut channel = TestClient::spawn(client, Default::default()).into_channel();
            assert_eq!(
                channel
                    .send(Request::new(1u8).to_untyped_request().unwrap())
                    .await
                    .unwrap()
                    .to_typed_response::<u8>()
                    .unwrap()
                    .payload,
                2
            );
        }

        #[test(tokio::test)]
        async fn should_attempt_to_reconnect_if_no_activity_from_server_within_silence_duration() {
            let (client, _) = Connection::pair(100);

            // NOTE: We consume the client to produce a channel without maintaining the shutdown
            //       channel in order to ensure that dropping the client does not kill the task.
            let client = TestClient::spawn(
                client,
                ClientConfig {
                    silence_duration: Duration::from_millis(100),
                    reconnect_strategy: ReconnectStrategy::FixedInterval {
                        interval: Duration::from_millis(50),
                        max_retries: Some(3),
                        timeout: None,
                    },
                    ..Default::default()
                },
            );

            let (tx, mut rx) = mpsc::unbounded_channel();
            client.on_connection_change(move |state| tx.send(state).unwrap());
            assert_eq!(rx.recv().await, Some(ConnectionState::Reconnecting));
            assert_eq!(rx.recv().await, Some(ConnectionState::Disconnected));
            assert_eq!(rx.recv().await, None);
        }
    }
}
