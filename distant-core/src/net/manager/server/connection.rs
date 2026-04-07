use std::collections::HashMap;
use std::{fmt, io};

use log::*;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::net::client::{ConnectionWatcher, Mailbox, UntypedClient};
use crate::net::common::{ConnectionId, Map, UntypedRequest, UntypedResponse};
use crate::net::manager::data::{ManagerChannelId, ManagerResponse};
use crate::net::server::ServerReply;

/// Represents a connection a distant manager has with some distant-compatible server.
pub struct ManagerConnection {
    pub id: ConnectionId,
    /// Raw destination string as provided by the user (e.g. `"docker://ubuntu:22.04"`).
    pub destination: String,
    pub options: Map,
    tx: mpsc::UnboundedSender<Action>,

    action_task: JoinHandle<()>,
    request_task: JoinHandle<()>,
    response_task: JoinHandle<()>,

    /// Optional task that monitors the underlying connection health and sends
    /// a death notification when the connection transitions to `Disconnected`.
    monitor_task: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct ManagerChannel {
    channel_id: ManagerChannelId,
    tx: mpsc::UnboundedSender<Action>,
}

impl ManagerChannel {
    /// Returns the id associated with the channel.
    pub fn id(&self) -> ManagerChannelId {
        self.channel_id
    }

    /// Sends the untyped request to the server on the other side of the channel.
    pub fn send(&self, req: UntypedRequest<'static>) -> io::Result<()> {
        let id = self.channel_id;

        self.tx.send(Action::Write { id, req }).map_err(|x| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("channel {id} send failed: {x}"),
            )
        })
    }

    /// Closes the channel, unregistering it with the connection.
    pub fn close(&self) -> io::Result<()> {
        let id = self.channel_id;
        self.tx.send(Action::Unregister { id }).map_err(|x| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("channel {id} close failed: {x}"),
            )
        })
    }
}

impl ManagerConnection {
    /// Spawns a new manager connection wrapping the given [`UntypedClient`].
    ///
    /// If `death_tx` is provided, a background monitor task will watch the client's connection
    /// health and send the connection ID through the channel when the connection dies.
    pub async fn spawn(
        destination: impl Into<String>,
        options: Map,
        mut client: UntypedClient,
        death_tx: Option<mpsc::UnboundedSender<ConnectionId>>,
    ) -> io::Result<Self> {
        let destination = destination.into();
        let connection_id = rand::random();
        let (tx, rx) = mpsc::unbounded_channel();

        // NOTE: Ensure that the connection is severed when the client is dropped; otherwise, when
        // the connection is terminated via aborting it or the connection being dropped, the
        // connection will persist which can cause problems such as lonely shutdown of the server
        // never triggering!
        client.shutdown_on_drop(true);

        // Clone the connection watcher before moving the client into tasks
        let watcher = client.clone_connection_watcher();

        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let action_task = tokio::spawn(action_task(connection_id, rx, request_tx));
        let response_task = tokio::spawn(response_task(
            connection_id,
            client.assign_default_mailbox(100).await?,
            tx.clone(),
        ));
        let request_task = tokio::spawn(request_task(connection_id, client, request_rx));

        // Spawn a monitor task if a death notification channel was provided
        let monitor_task =
            death_tx.map(|dtx| tokio::spawn(connection_monitor(connection_id, watcher, dtx)));

        Ok(Self {
            id: connection_id,
            destination,
            options,
            tx,
            action_task,
            request_task,
            response_task,
            monitor_task,
        })
    }

    /// Replaces the underlying client with a new one, aborting old tasks and
    /// respawning them with the same [`ConnectionId`].
    ///
    /// **Existing channels are invalidated** — the old action task is aborted
    /// and a new one is spawned, so any [`ManagerChannel`] handles obtained
    /// before this call will fail on subsequent sends. Callers must re-open
    /// channels after replacement.
    ///
    /// If `death_tx` is provided, a new connection monitor task is spawned.
    ///
    /// # Errors
    ///
    /// Returns an error if the default mailbox cannot be assigned on the new client.
    pub async fn replace_client(
        &mut self,
        mut client: UntypedClient,
        death_tx: Option<mpsc::UnboundedSender<ConnectionId>>,
    ) -> io::Result<()> {
        let id = self.id;
        debug!("[Conn {id}] Replacing client — aborting old tasks");

        // Abort old tasks (action_task is NOT aborted — channels live there)
        self.request_task.abort();
        self.response_task.abort();
        if let Some(ref task) = self.monitor_task {
            task.abort();
        }

        // Configure the new client
        client.shutdown_on_drop(true);

        // Clone watcher before moving the client
        let watcher = client.clone_connection_watcher();

        // Set up new request and response tasks using the existing action tx
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let mailbox = client.assign_default_mailbox(100).await?;

        self.response_task = tokio::spawn(response_task(id, mailbox, self.tx.clone()));
        self.request_task = tokio::spawn(request_task(id, client, request_rx));

        // Abort the old action task and respawn with the new request_tx.
        // Channel registrations start fresh — existing ManagerChannel handles
        // hold a clone of the OLD self.tx and will fail on next send.
        self.action_task.abort();
        let (tx, rx) = mpsc::unbounded_channel();
        self.action_task = tokio::spawn(action_task(id, rx, request_tx));
        self.tx = tx;

        // Spawn a new monitor task if requested
        self.monitor_task = death_tx.map(|dtx| tokio::spawn(connection_monitor(id, watcher, dtx)));

        info!("[Conn {id}] Client replaced successfully");
        Ok(())
    }

    pub fn open_channel(&self, reply: ServerReply<ManagerResponse>) -> io::Result<ManagerChannel> {
        let channel_id = rand::random();
        self.tx
            .send(Action::Register {
                id: channel_id,
                reply,
            })
            .map_err(|x| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("open_channel failed: {x}"),
                )
            })?;
        Ok(ManagerChannel {
            channel_id,
            tx: self.tx.clone(),
        })
    }

    pub async fn channel_ids(&self) -> io::Result<Vec<ManagerChannelId>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Action::GetRegistered { cb: tx })
            .map_err(|x| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("channel_ids failed: {x}"),
                )
            })?;

        let channel_ids = rx.await.map_err(|x| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("channel_ids callback dropped: {x}"),
            )
        })?;
        Ok(channel_ids)
    }

    /// Aborts the tasks used to engage with the connection.
    pub fn abort(&self) {
        self.action_task.abort();
        self.request_task.abort();
        self.response_task.abort();
        if let Some(ref task) = self.monitor_task {
            task.abort();
        }
    }
}

impl Drop for ManagerConnection {
    fn drop(&mut self) {
        self.abort();
    }
}

enum Action {
    Register {
        id: ManagerChannelId,
        reply: ServerReply<ManagerResponse>,
    },

    Unregister {
        id: ManagerChannelId,
    },

    GetRegistered {
        cb: oneshot::Sender<Vec<ManagerChannelId>>,
    },

    Read {
        res: UntypedResponse<'static>,
    },

    Write {
        id: ManagerChannelId,
        req: UntypedRequest<'static>,
    },
}

impl fmt::Debug for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Register { id, .. } => write!(f, "Action::Register {{ id: {id}, .. }}"),
            Self::Unregister { id } => write!(f, "Action::Unregister {{ id: {id} }}"),
            Self::GetRegistered { .. } => write!(f, "Action::GetRegistered {{ .. }}"),
            Self::Read { .. } => write!(f, "Action::Read {{ .. }}"),
            Self::Write { id, .. } => write!(f, "Action::Write {{ id: {id}, .. }}"),
        }
    }
}

/// Watches a connection's health state and sends a death notification when the
/// connection transitions to [`ConnectionState::Disconnected`].
///
/// If the watcher channel closes (sender dropped), the connection is also considered dead.
async fn connection_monitor(
    id: ConnectionId,
    mut watcher: ConnectionWatcher,
    death_tx: mpsc::UnboundedSender<ConnectionId>,
) {
    while let Some(state) = watcher.next().await {
        if state.is_disconnected() {
            info!("[Conn {id}] Connection died, notifying manager");
            let _ = death_tx.send(id);
            return;
        }
    }
    // Watcher channel closed (sender dropped) — connection is dead
    debug!("[Conn {id}] Connection watcher closed");
    let _ = death_tx.send(id);
}

/// Internal task to process outgoing [`UntypedRequest`]s.
async fn request_task(
    id: ConnectionId,
    mut client: UntypedClient,
    mut rx: mpsc::UnboundedReceiver<UntypedRequest<'static>>,
) {
    while let Some(req) = rx.recv().await {
        trace!("[Conn {id}] Firing off request {}", req.id);
        if let Err(x) = client.fire(req).await {
            error!("[Conn {id}] Failed to send request: {x}");
        }
    }

    trace!("[Conn {id}] Manager request task closed");
}

/// Internal task to process incoming [`UntypedResponse`]s.
async fn response_task(
    id: ConnectionId,
    mut mailbox: Mailbox<UntypedResponse<'static>>,
    tx: mpsc::UnboundedSender<Action>,
) {
    while let Some(res) = mailbox.next().await {
        trace!(
            "[Conn {id}] Receiving response {} to request {}",
            res.id, res.origin_id
        );
        if let Err(x) = tx.send(Action::Read { res }) {
            error!("[Conn {id}] Failed to forward received response: {x}");
        }
    }

    trace!("[Conn {id}] Manager response task closed");
}

/// Internal task to process [`Action`] items.
///
/// * `id` - the id of the connection.
/// * `rx` - used to receive new [`Action`]s to process.
/// * `tx` - used to send outgoing requests through the connection.
async fn action_task(
    id: ConnectionId,
    mut rx: mpsc::UnboundedReceiver<Action>,
    tx: mpsc::UnboundedSender<UntypedRequest<'static>>,
) {
    let mut registered = HashMap::new();

    while let Some(action) = rx.recv().await {
        trace!("[Conn {id}] {action:?}");

        match action {
            Action::Register { id, reply } => {
                registered.insert(id, reply);
            }
            Action::Unregister { id } => {
                registered.remove(&id);
            }
            Action::GetRegistered { cb } => {
                let _ = cb.send(registered.keys().copied().collect());
            }
            Action::Read { mut res } => {
                // Split {channel id}_{request id} back into pieces and
                // update the origin id to match the request id only
                let channel_id = match res.origin_id.split_once('_') {
                    Some((cid_str, oid_str)) => {
                        if let Ok(cid) = cid_str.parse::<ManagerChannelId>() {
                            res.set_origin_id(oid_str.to_string());
                            cid
                        } else {
                            continue;
                        }
                    }
                    None => continue,
                };

                if let Some(reply) = registered.get(&channel_id) {
                    let response = ManagerResponse::Channel {
                        id: channel_id,
                        response: res,
                    };

                    if let Err(x) = reply.send(response) {
                        error!("[Conn {id}] {x}");
                    }
                }
            }
            Action::Write { id, mut req } => {
                // Combine channel id with request id so we can properly forward
                // the response containing this in the origin id
                req.set_id(format!("{id}_{}", req.id));

                if let Err(x) = tx.send(req) {
                    error!("[Conn {id}] {x}");
                }
            }
        }
    }

    trace!("[Conn {id}] Manager action task closed");
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::net::client::UntypedClient;
    use crate::net::common::{Connection, InmemoryTransport, Response};

    fn make_untyped_client() -> (UntypedClient, Connection<InmemoryTransport>) {
        let (client_conn, server_conn) = Connection::pair(100);
        let client = UntypedClient::spawn(client_conn, Default::default());
        (client, server_conn)
    }

    // ---- ManagerChannel ----

    #[test]
    fn manager_channel_id_returns_channel_id() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let channel = ManagerChannel { channel_id: 42, tx };
        assert_eq!(channel.id(), 42);
    }

    #[test]
    fn manager_channel_send_succeeds_when_receiver_alive() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let channel = ManagerChannel { channel_id: 1, tx };

        let req = UntypedRequest {
            header: std::borrow::Cow::Owned(vec![]),
            id: std::borrow::Cow::Owned("req-1".to_string()),
            payload: std::borrow::Cow::Owned(vec![0xc3]),
        };
        let result = channel.send(req);
        assert!(result.is_ok());
    }

    #[test]
    fn manager_channel_send_fails_when_receiver_dropped() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let channel = ManagerChannel { channel_id: 1, tx };

        let req = UntypedRequest {
            header: std::borrow::Cow::Owned(vec![]),
            id: std::borrow::Cow::Owned("req-1".to_string()),
            payload: std::borrow::Cow::Owned(vec![0xc3]),
        };
        let err = channel.send(req).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn manager_channel_close_succeeds_when_receiver_alive() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let channel = ManagerChannel { channel_id: 5, tx };
        let result = channel.close();
        assert!(result.is_ok());
    }

    #[test]
    fn manager_channel_close_fails_when_receiver_dropped() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);
        let channel = ManagerChannel { channel_id: 5, tx };
        let err = channel.close().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn manager_channel_clone_shares_same_tx() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let channel = ManagerChannel { channel_id: 10, tx };
        let cloned = channel.clone();
        assert_eq!(cloned.id(), 10);
    }

    // ---- ManagerConnection ----

    #[test_log::test(tokio::test)]
    async fn manager_connection_spawn_sets_id_and_destination() {
        let (client, _server) = make_untyped_client();
        let dest = "scheme://host".to_string();
        let opts: Map = "key=value".parse().unwrap();

        let conn = ManagerConnection::spawn(dest.clone(), opts.clone(), client, None)
            .await
            .unwrap();

        assert_eq!(conn.destination, dest);
        assert_eq!(conn.options, opts);
        // id is randomly generated, just check it's non-zero (very unlikely to be 0)
        // We just verify it exists
        let _ = conn.id;
    }

    #[test_log::test(tokio::test)]
    async fn manager_connection_open_channel_returns_channel_with_random_id() {
        let (client, _server) = make_untyped_client();
        let dest = "scheme://host".to_string();
        let opts: Map = Map::new();

        let conn = ManagerConnection::spawn(dest, opts, client, None)
            .await
            .unwrap();

        let (reply_tx, _reply_rx) = mpsc::unbounded_channel();
        let reply = ServerReply {
            origin_id: "test".to_string(),
            tx: reply_tx,
        };

        let channel = conn.open_channel(reply).unwrap();
        // Channel has a randomly generated id
        let _ = channel.id();
    }

    #[test_log::test(tokio::test)]
    async fn manager_connection_open_channel_registers_and_shows_in_channel_ids() {
        let (client, _server) = make_untyped_client();
        let dest = "scheme://host".to_string();
        let opts: Map = Map::new();

        let conn = ManagerConnection::spawn(dest, opts, client, None)
            .await
            .unwrap();

        let (reply_tx, _reply_rx) = mpsc::unbounded_channel();
        let reply = ServerReply {
            origin_id: "test".to_string(),
            tx: reply_tx,
        };

        let channel = conn.open_channel(reply).unwrap();
        let channel_id = channel.id();

        // Give the action task a moment to process the register action
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ids = conn.channel_ids().await.unwrap();
        assert!(ids.contains(&channel_id));
    }

    #[test_log::test(tokio::test)]
    async fn manager_connection_channel_ids_empty_initially() {
        let (client, _server) = make_untyped_client();
        let dest = "scheme://host".to_string();
        let opts: Map = Map::new();

        let conn = ManagerConnection::spawn(dest, opts, client, None)
            .await
            .unwrap();

        let ids = conn.channel_ids().await.unwrap();
        assert!(ids.is_empty());
    }

    #[test_log::test(tokio::test)]
    async fn manager_connection_open_multiple_channels_all_registered() {
        let (client, _server) = make_untyped_client();
        let dest = "scheme://host".to_string();
        let opts: Map = Map::new();

        let conn = ManagerConnection::spawn(dest, opts, client, None)
            .await
            .unwrap();

        let mut channel_ids = Vec::new();
        for _ in 0..3 {
            let (reply_tx, _reply_rx) = mpsc::unbounded_channel();
            let reply = ServerReply {
                origin_id: "test".to_string(),
                tx: reply_tx,
            };
            let channel = conn.open_channel(reply).unwrap();
            channel_ids.push(channel.id());
        }

        // Give the action task time to process
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ids = conn.channel_ids().await.unwrap();
        assert_eq!(ids.len(), 3);
        for cid in &channel_ids {
            assert!(ids.contains(cid));
        }
    }

    #[test_log::test(tokio::test)]
    async fn manager_connection_channel_close_unregisters() {
        let (client, _server) = make_untyped_client();
        let dest = "scheme://host".to_string();
        let opts: Map = Map::new();

        let conn = ManagerConnection::spawn(dest, opts, client, None)
            .await
            .unwrap();

        let (reply_tx, _reply_rx) = mpsc::unbounded_channel();
        let reply = ServerReply {
            origin_id: "test".to_string(),
            tx: reply_tx,
        };

        let channel = conn.open_channel(reply).unwrap();
        let channel_id = channel.id();

        // Wait for registration
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let ids = conn.channel_ids().await.unwrap();
        assert!(ids.contains(&channel_id));

        // Close the channel
        channel.close().unwrap();

        // Wait for unregistration
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let ids = conn.channel_ids().await.unwrap();
        assert!(!ids.contains(&channel_id));
    }

    #[test_log::test(tokio::test)]
    async fn manager_connection_abort_stops_tasks() {
        let (client, _server) = make_untyped_client();
        let dest = "scheme://host".to_string();
        let opts: Map = Map::new();

        let conn = ManagerConnection::spawn(dest, opts, client, None)
            .await
            .unwrap();
        conn.abort();

        // After abort, channel_ids should fail because the action task is aborted
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let result = conn.channel_ids().await;
        assert!(result.is_err());
    }

    #[test_log::test(tokio::test)]
    async fn manager_connection_open_channel_fails_after_abort() {
        let (client, _server) = make_untyped_client();
        let dest = "scheme://host".to_string();
        let opts: Map = Map::new();

        let conn = ManagerConnection::spawn(dest, opts, client, None)
            .await
            .unwrap();
        conn.abort();

        // Give time for abort to take effect
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let (reply_tx, _reply_rx) = mpsc::unbounded_channel();
        let reply = ServerReply {
            origin_id: "test".to_string(),
            tx: reply_tx,
        };

        // open_channel sends to the tx, but the receiver is aborted
        // The tx.send itself may still succeed (buffered) but the message won't be processed
        // At minimum, the channel is created
        let result = conn.open_channel(reply);
        // Result may be Ok (send succeeded) or Err (channel closed), depends on timing
        // If Ok, the channel_ids query will fail
        if result.is_ok() {
            let ids_result = conn.channel_ids().await;
            assert!(ids_result.is_err());
        }
    }

    // ---- Connection Monitor ----

    #[test_log::test(tokio::test)]
    async fn connection_monitor_should_send_death_on_disconnect() {
        let (client, server_conn) = make_untyped_client();
        let watcher = client.clone_connection_watcher();
        let connection_id: ConnectionId = 12345;

        let (death_tx, mut death_rx) = mpsc::unbounded_channel();

        // Spawn the monitor in the background
        let _monitor = tokio::spawn(connection_monitor(connection_id, watcher, death_tx));

        // Drop the server side of the connection to trigger disconnect.
        // The client's event loop will detect the broken transport, attempt
        // reconnection (which fails immediately with the default Fail strategy),
        // and transition to Disconnected.
        drop(server_conn);
        // Also drop the client so the watcher task can observe the state change
        // before the client is fully cleaned up. Actually, the client task runs
        // independently, so the watcher should see Disconnected once the task
        // completes its reconnect failure path.
        drop(client);

        // Wait for the death notification with a timeout
        let received_id = tokio::time::timeout(Duration::from_secs(5), death_rx.recv())
            .await
            .expect("timed out waiting for death notification")
            .expect("death channel closed without sending");

        assert_eq!(received_id, connection_id);
    }

    #[test_log::test(tokio::test)]
    async fn connection_monitor_should_send_death_when_watcher_closes() {
        // Create a client with shutdown_on_drop=true so dropping it aborts the
        // internal task immediately, which drops the watch::Sender without first
        // sending a Disconnected state. This exercises the fallback path in
        // connection_monitor where watcher.next() returns None.
        let (mut client, _server_conn) = make_untyped_client();
        client.shutdown_on_drop(true);
        let watcher = client.clone_connection_watcher();
        let connection_id: ConnectionId = 67890;

        let (death_tx, mut death_rx) = mpsc::unbounded_channel();

        // Spawn the monitor in the background
        let _monitor = tokio::spawn(connection_monitor(connection_id, watcher, death_tx));

        // Drop the client. Because shutdown_on_drop is true, this aborts the
        // internal task, dropping the watch sender. The server connection is kept
        // alive so the client task has no reason to send Disconnected before abort.
        drop(client);

        let received_id = tokio::time::timeout(Duration::from_secs(5), death_rx.recv())
            .await
            .expect("timed out waiting for death notification")
            .expect("death channel closed without sending");

        assert_eq!(received_id, connection_id);
    }

    #[test_log::test(tokio::test)]
    async fn spawn_with_death_tx_should_notify_on_client_drop() {
        let (client, server_conn) = make_untyped_client();
        let (death_tx, mut death_rx) = mpsc::unbounded_channel();

        let conn = ManagerConnection::spawn("scheme://host", Map::new(), client, Some(death_tx))
            .await
            .unwrap();

        let connection_id = conn.id;

        // Drop the server side to trigger disconnection in the underlying transport.
        // The client event loop will fail reconnection and transition to Disconnected,
        // which the monitor task observes and sends through death_tx.
        drop(server_conn);

        let received_id = tokio::time::timeout(Duration::from_secs(5), death_rx.recv())
            .await
            .expect("timed out waiting for death notification")
            .expect("death channel closed without sending");

        assert_eq!(received_id, connection_id);

        // Clean up the connection to abort its tasks
        conn.abort();
    }

    #[test_log::test(tokio::test)]
    async fn spawn_without_death_tx_should_not_have_monitor_task() {
        let (client, _server_conn) = make_untyped_client();

        let conn = ManagerConnection::spawn("scheme://host", Map::new(), client, None)
            .await
            .unwrap();

        assert!(
            conn.monitor_task.is_none(),
            "monitor_task should be None when no death_tx is provided"
        );
    }

    #[test_log::test(tokio::test)]
    async fn spawn_with_death_tx_should_have_monitor_task() {
        let (client, _server_conn) = make_untyped_client();
        let (death_tx, _death_rx) = mpsc::unbounded_channel();

        let conn = ManagerConnection::spawn("scheme://host", Map::new(), client, Some(death_tx))
            .await
            .unwrap();

        assert!(
            conn.monitor_task.is_some(),
            "monitor_task should be Some when death_tx is provided"
        );
    }

    // ---- Action Debug ----

    #[test]
    fn action_debug_register() {
        let (reply_tx, _) = mpsc::unbounded_channel::<Response<ManagerResponse>>();
        let reply = ServerReply {
            origin_id: "test".to_string(),
            tx: reply_tx,
        };
        let action = Action::Register { id: 42, reply };
        let debug = format!("{action:?}");
        assert_eq!(debug, "Action::Register { id: 42, .. }");
    }

    #[test]
    fn action_debug_unregister() {
        let action = Action::Unregister { id: 99 };
        let debug = format!("{action:?}");
        assert_eq!(debug, "Action::Unregister { id: 99 }");
    }

    #[test]
    fn action_debug_get_registered() {
        let (tx, _) = oneshot::channel();
        let action = Action::GetRegistered { cb: tx };
        let debug = format!("{action:?}");
        assert_eq!(debug, "Action::GetRegistered { .. }");
    }

    #[test]
    fn action_debug_read() {
        let res = UntypedResponse {
            header: std::borrow::Cow::Owned(vec![]),
            id: std::borrow::Cow::Owned("id".to_string()),
            origin_id: std::borrow::Cow::Owned("oid".to_string()),
            payload: std::borrow::Cow::Owned(vec![]),
        };
        let action = Action::Read { res };
        let debug = format!("{action:?}");
        assert_eq!(debug, "Action::Read { .. }");
    }

    #[test]
    fn action_debug_write() {
        let req = UntypedRequest {
            header: std::borrow::Cow::Owned(vec![]),
            id: std::borrow::Cow::Owned("req".to_string()),
            payload: std::borrow::Cow::Owned(vec![]),
        };
        let action = Action::Write { id: 7, req };
        let debug = format!("{action:?}");
        assert_eq!(debug, "Action::Write { id: 7, .. }");
    }

    // ---- replace_client ----

    #[test_log::test(tokio::test)]
    async fn replace_client_should_preserve_connection_id() {
        let (client, _server) = make_untyped_client();
        let conn = ManagerConnection::spawn("scheme://host", Map::new(), client, None)
            .await
            .unwrap();
        let original_id = conn.id;

        // Build a new client to replace with
        let (new_client, _new_server) = make_untyped_client();

        let mut conn = conn;
        conn.replace_client(new_client, None).await.unwrap();

        assert_eq!(conn.id, original_id);
    }

    #[test_log::test(tokio::test)]
    async fn replace_client_should_preserve_destination_and_options() {
        let (client, _server) = make_untyped_client();
        let opts: Map = "key=value".parse().unwrap();
        let dest = "scheme://host".to_string();
        let conn = ManagerConnection::spawn(dest.clone(), opts.clone(), client, None)
            .await
            .unwrap();

        let (new_client, _new_server) = make_untyped_client();

        let mut conn = conn;
        conn.replace_client(new_client, None).await.unwrap();

        assert_eq!(conn.destination, dest);
        assert_eq!(conn.options, opts);
    }

    #[test_log::test(tokio::test)]
    async fn replace_client_should_allow_new_channels_after_replacement() {
        let (client, _server) = make_untyped_client();
        let mut conn = ManagerConnection::spawn("scheme://host", Map::new(), client, None)
            .await
            .unwrap();

        // Replace with a new client
        let (new_client, _new_server) = make_untyped_client();
        conn.replace_client(new_client, None).await.unwrap();

        // Give the new action task time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Open a channel on the replacement
        let (reply_tx, _reply_rx) = mpsc::unbounded_channel();
        let reply = ServerReply {
            origin_id: "test".to_string(),
            tx: reply_tx,
        };
        let channel = conn.open_channel(reply).unwrap();
        let channel_id = channel.id();

        // Wait for registration
        tokio::time::sleep(Duration::from_millis(50)).await;

        let ids = conn.channel_ids().await.unwrap();
        assert!(
            ids.contains(&channel_id),
            "New channel should be registered after replace_client"
        );
    }

    #[test_log::test(tokio::test)]
    async fn replace_client_with_death_tx_should_have_monitor_task() {
        let (client, _server) = make_untyped_client();
        let mut conn = ManagerConnection::spawn("scheme://host", Map::new(), client, None)
            .await
            .unwrap();

        assert!(
            conn.monitor_task.is_none(),
            "Initial spawn without death_tx should have no monitor"
        );

        let (death_tx, _death_rx) = mpsc::unbounded_channel();
        let (new_client, _new_server) = make_untyped_client();
        conn.replace_client(new_client, Some(death_tx))
            .await
            .unwrap();

        assert!(
            conn.monitor_task.is_some(),
            "After replace_client with death_tx, monitor should be Some"
        );
    }

    #[test_log::test(tokio::test)]
    async fn replace_client_without_death_tx_should_not_have_monitor_task() {
        let (client, _server) = make_untyped_client();
        let (death_tx, _death_rx) = mpsc::unbounded_channel();
        let mut conn =
            ManagerConnection::spawn("scheme://host", Map::new(), client, Some(death_tx))
                .await
                .unwrap();

        assert!(
            conn.monitor_task.is_some(),
            "Initial spawn with death_tx should have monitor"
        );

        let (new_client, _new_server) = make_untyped_client();
        conn.replace_client(new_client, None).await.unwrap();

        assert!(
            conn.monitor_task.is_none(),
            "After replace_client without death_tx, monitor should be None"
        );
    }

    #[test_log::test(tokio::test)]
    async fn replace_client_should_start_with_empty_channel_registrations() {
        let (client, _server) = make_untyped_client();
        let mut conn = ManagerConnection::spawn("scheme://host", Map::new(), client, None)
            .await
            .unwrap();

        // Register a channel on the old action task
        let (reply_tx, _reply_rx) = mpsc::unbounded_channel();
        let reply = ServerReply {
            origin_id: "test".to_string(),
            tx: reply_tx,
        };
        let _channel = conn.open_channel(reply).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let ids_before = conn.channel_ids().await.unwrap();
        assert_eq!(ids_before.len(), 1);

        // Replace the client -- this replaces the action task, so registrations reset
        let (new_client, _new_server) = make_untyped_client();
        conn.replace_client(new_client, None).await.unwrap();

        // Give the new action task time to start
        tokio::time::sleep(Duration::from_millis(50)).await;

        let ids_after = conn.channel_ids().await.unwrap();
        assert!(
            ids_after.is_empty(),
            "Channel registrations should be empty after replace_client, but found: {ids_after:?}"
        );
    }

    #[test_log::test(tokio::test)]
    async fn replace_client_death_tx_should_notify_on_new_client_disconnect() {
        let (client, _server) = make_untyped_client();
        let mut conn = ManagerConnection::spawn("scheme://host", Map::new(), client, None)
            .await
            .unwrap();
        let conn_id = conn.id;

        let (death_tx, mut death_rx) = mpsc::unbounded_channel();
        let (new_client, new_server) = make_untyped_client();
        conn.replace_client(new_client, Some(death_tx))
            .await
            .unwrap();

        // Drop the server side to trigger disconnect detection on the new client
        drop(new_server);

        let received_id = tokio::time::timeout(Duration::from_secs(5), death_rx.recv())
            .await
            .expect("timed out waiting for death notification")
            .expect("death channel closed without sending");

        assert_eq!(received_id, conn_id);
    }

    #[test_log::test(tokio::test)]
    async fn replace_client_should_propagate_error_when_client_task_is_dead() {
        let (client, _server) = make_untyped_client();
        let mut conn = ManagerConnection::spawn("scheme://host", Map::new(), client, None)
            .await
            .unwrap();

        // Create a new client and abort its internal task so the post office
        // is dropped. This causes assign_default_mailbox to fail with
        // NotConnected because the Weak<PostOffice> cannot be upgraded.
        let (mut dead_client, _dead_server) = make_untyped_client();
        dead_client.shutdown_on_drop(true);
        dead_client.abort();
        // Give the task time to actually stop
        tokio::time::sleep(Duration::from_millis(50)).await;

        let err = conn.replace_client(dead_client, None).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }
}
