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
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

mod builder;
pub use builder::*;

mod channel;
pub use channel::*;

mod reconnect;
pub use reconnect::*;

mod shutdown;
pub use shutdown::*;

/// Time to wait inbetween connection read/write when nothing was read or written on last pass
const SLEEP_DURATION: Duration = Duration::from_millis(50);

/// Represents a client that can be used to send requests & receive responses from a server.
pub struct Client<T, U> {
    /// Used to send requests to a server.
    channel: Channel<T, U>,

    /// Used to send shutdown request to inner task.
    shutdown: Box<dyn Shutdown>,

    /// Contains the task that is running to send requests and receive responses from a server.
    task: JoinHandle<io::Result<()>>,
}

impl<T, U> Client<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Spawns a client using the provided [`Connection`].
    pub(crate) fn spawn<V>(mut connection: Connection<V>, mut strategy: ReconnectStrategy) -> Self
    where
        V: Transport + Send + Sync + 'static,
    {
        let post_office = Arc::new(PostOffice::default());
        let weak_post_office = Arc::downgrade(&post_office);
        let (tx, mut rx) = mpsc::channel::<Request<T>>(1);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<oneshot::Sender<io::Result<()>>>(1);

        // Ensure that our transport starts off clean (nothing in buffers or backup)
        connection.clear();

        // Start a task that continually checks for responses and delivers them using the
        // post office
        let shutdown_tx_2 = shutdown_tx.clone();
        let task = tokio::spawn(async move {
            let mut needs_reconnect = false;

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
                        debug!("Using strategy {strategy:?}");
                    }
                    match strategy.reconnect(&mut connection).await {
                        Ok(x) => {
                            needs_reconnect = false;
                            x
                        }
                        Err(x) => {
                            error!("Unable to re-establish connection: {x}");
                            break Err(x);
                        }
                    }
                }

                let ready = tokio::select! {
                    // NOTE: This should NEVER return None as we never allow the channel to close.
                    cb = shutdown_rx.recv() => {
                        debug!("Client got shutdown signal, so exiting event loop");
                        let cb = cb.expect("Impossible: shutdown channel closed!");
                        let _ = cb.send(Ok(()));
                        break Ok(());
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
                            needs_reconnect = true;
                            continue;
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
                                    error!("Failed to write frame: {x}");
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
            shutdown: Box::new(shutdown_tx),
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
    /// Convert into underlying channel.
    pub fn into_channel(self) -> Channel<T, U> {
        self.channel
    }

    /// Clones the underlying channel for requests and returns the cloned instance.
    pub fn clone_channel(&self) -> Channel<T, U> {
        self.channel.clone()
    }

    /// Waits for the client to terminate, which resolves when the receiving end of the network
    /// connection is closed (or the client is shutdown). Returns whether or not the client exited
    /// successfully or due to an error.
    pub async fn wait(self) -> io::Result<()> {
        match self.task.await {
            Ok(x) => x,
            Err(x) => Err(io::Error::new(io::ErrorKind::Other, x)),
        }
    }

    /// Abort the client's current connection by forcing its tasks to abort.
    pub fn abort(&self) {
        self.task.abort();
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

    /// Returns true if client's underlying event processing has finished/terminated.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::{Ready, Response, TestTransport};
    use test_log::test;

    type TestClient = Client<u8, u8>;

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

        let mut client = TestClient::spawn(client, ReconnectStrategy::Fail);
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

        let mut client = TestClient::spawn(client, ReconnectStrategy::Fail);
        assert_eq!(client.send(Request::new(1u8)).await.unwrap().payload, 2);
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
        let client = TestClient::spawn(client, ReconnectStrategy::Fail);
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

        let client = TestClient::spawn(client, ReconnectStrategy::Fail);
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
        let mut channel = TestClient::spawn(client, ReconnectStrategy::Fail).into_channel();
        assert_eq!(channel.send(Request::new(1u8)).await.unwrap().payload, 2);
    }
}
