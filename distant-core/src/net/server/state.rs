use std::collections::HashMap;

use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::net::common::{Backup, ConnectionId, Keychain};

/// Contains all top-level state for the server
pub struct ServerState<T> {
    /// Mapping of connection ids to their tasks.
    pub connections: RwLock<HashMap<ConnectionId, ConnectionState<T>>>,

    /// Mapping of connection ids to (OTP, backup)
    pub keychain: Keychain<oneshot::Receiver<Backup>>,
}

impl<T> ServerState<T> {
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            keychain: Keychain::new(),
        }
    }
}

impl<T> Default for ServerState<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ConnectionState<T> {
    shutdown_tx: oneshot::Sender<()>,
    task: JoinHandle<Option<(mpsc::UnboundedSender<T>, mpsc::UnboundedReceiver<T>)>>,
}

impl<T: Send + 'static> ConnectionState<T> {
    /// Creates new state with appropriate channels, returning
    /// (shutdown receiver, channel sender, state).
    #[allow(clippy::type_complexity)]
    pub fn channel() -> (
        oneshot::Receiver<()>,
        oneshot::Sender<(mpsc::UnboundedSender<T>, mpsc::UnboundedReceiver<T>)>,
        Self,
    ) {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (channel_tx, channel_rx) = oneshot::channel();

        (
            shutdown_rx,
            channel_tx,
            Self {
                shutdown_tx,
                task: tokio::spawn(async move { (channel_rx.await).ok() }),
            },
        )
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    pub async fn shutdown_and_wait(
        self,
    ) -> Option<(mpsc::UnboundedSender<T>, mpsc::UnboundedReceiver<T>)> {
        let _ = self.shutdown_tx.send(());
        self.task.await.unwrap()
    }
}

#[cfg(test)]
mod tests {
    //! Tests for ServerState<T> (connection map operations) and ConnectionState<T> (channel
    //! delivery, shutdown signaling, is_finished lifecycle, and shutdown_and_wait).

    use super::*;

    // ---------------------------------------------------------------
    // ServerState
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn server_state_new_starts_with_empty_connections() {
        let state = ServerState::<String>::new();
        let connections = state.connections.read().await;
        assert!(connections.is_empty());
    }

    #[test_log::test(tokio::test)]
    async fn server_state_default_starts_with_empty_connections() {
        let state = ServerState::<String>::default();
        let connections = state.connections.read().await;
        assert!(connections.is_empty());
    }

    #[test_log::test(tokio::test)]
    async fn server_state_can_insert_and_retrieve_connection() {
        let state = ServerState::<String>::new();
        let (_shutdown_rx, _channel_tx, conn_state) = ConnectionState::<String>::channel();

        {
            let mut connections = state.connections.write().await;
            connections.insert(42, conn_state);
        }

        let connections = state.connections.read().await;
        assert_eq!(connections.len(), 1);
        assert!(connections.contains_key(&42));
    }

    // ---------------------------------------------------------------
    // ConnectionState::channel
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn channel_returns_non_finished_state() {
        let (_shutdown_rx, _channel_tx, state) = ConnectionState::<String>::channel();
        assert!(!state.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn channel_shutdown_receiver_gets_signal_on_shutdown() {
        let (shutdown_rx, _channel_tx, state) = ConnectionState::<String>::channel();

        // Trigger shutdown
        let _ = state.shutdown_tx.send(());

        // The shutdown receiver should complete
        assert!(shutdown_rx.await.is_ok());
    }

    #[test_log::test(tokio::test)]
    async fn channel_sender_delivers_channels_to_task() {
        let (_shutdown_rx, channel_tx, state) = ConnectionState::<String>::channel();

        let (tx, rx) = mpsc::unbounded_channel();
        channel_tx.send((tx, rx)).unwrap();

        // The task should resolve with Some
        let result = state.task.await.unwrap();
        assert!(result.is_some());
    }

    #[test_log::test(tokio::test)]
    async fn channel_task_returns_none_when_sender_dropped() {
        let (_shutdown_rx, channel_tx, state) = ConnectionState::<String>::channel();

        // Drop the channel sender without sending
        drop(channel_tx);

        // The task should resolve with None
        let result = state.task.await.unwrap();
        assert!(result.is_none());
    }

    // ---------------------------------------------------------------
    // ConnectionState::is_finished
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn is_finished_returns_false_while_task_running() {
        let (_shutdown_rx, _channel_tx, state) = ConnectionState::<String>::channel();
        assert!(!state.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn is_finished_returns_true_after_task_completes() {
        let (_shutdown_rx, channel_tx, state) = ConnectionState::<String>::channel();

        // Drop sender to complete the task
        drop(channel_tx);

        // Wait a bit for the task to finish
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(state.is_finished());
    }

    // ---------------------------------------------------------------
    // ConnectionState::shutdown_and_wait
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn shutdown_and_wait_returns_none_when_no_channels_provided() {
        let (_shutdown_rx, channel_tx, state) = ConnectionState::<String>::channel();

        // Drop the channel sender without sending channels
        drop(channel_tx);

        let result = state.shutdown_and_wait().await;
        assert!(result.is_none());
    }

    #[test_log::test(tokio::test)]
    async fn shutdown_and_wait_returns_channels_when_provided() {
        let (_shutdown_rx, channel_tx, state) = ConnectionState::<String>::channel();

        let (tx, rx) = mpsc::unbounded_channel();
        channel_tx.send((tx, rx)).unwrap();

        let result = state.shutdown_and_wait().await;
        assert!(result.is_some());

        let (sender, mut receiver) = result.unwrap();
        // Verify the channels work
        sender.send(String::from("hello")).unwrap();
        let msg = receiver.recv().await.unwrap();
        assert_eq!(msg, "hello");
    }

    // ---------------------------------------------------------------
    // Integration: ServerState with ConnectionState
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn server_state_can_shutdown_connection() {
        let state = ServerState::<String>::new();

        let (_shutdown_rx, channel_tx, conn_state) = ConnectionState::<String>::channel();
        let (tx, rx) = mpsc::unbounded_channel();
        channel_tx.send((tx, rx)).unwrap();

        {
            let mut connections = state.connections.write().await;
            connections.insert(1, conn_state);
        }

        let conn_state = {
            let mut connections = state.connections.write().await;
            connections.remove(&1).unwrap()
        };

        let result = conn_state.shutdown_and_wait().await;
        assert!(result.is_some());

        let connections = state.connections.read().await;
        assert!(connections.is_empty());
    }
}
