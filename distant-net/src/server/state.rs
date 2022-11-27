use crate::common::{authentication::Keychain, Backup, ConnectionId};
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;

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
    task: JoinHandle<Option<(mpsc::Sender<T>, mpsc::Receiver<T>)>>,
}

impl<T: Send + 'static> ConnectionState<T> {
    /// Creates new state with appropriate channels, returning
    /// (shutdown receiver, channel sender, state).
    #[allow(clippy::type_complexity)]
    pub fn channel() -> (
        oneshot::Receiver<()>,
        oneshot::Sender<(mpsc::Sender<T>, mpsc::Receiver<T>)>,
        Self,
    ) {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (channel_tx, channel_rx) = oneshot::channel();

        (
            shutdown_rx,
            channel_tx,
            Self {
                shutdown_tx,
                task: tokio::spawn(async move {
                    match channel_rx.await {
                        Ok(x) => Some(x),
                        Err(_) => None,
                    }
                }),
            },
        )
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    pub async fn shutdown_and_wait(self) -> Option<(mpsc::Sender<T>, mpsc::Receiver<T>)> {
        let _ = self.shutdown_tx.send(());
        self.task.await.unwrap()
    }
}
