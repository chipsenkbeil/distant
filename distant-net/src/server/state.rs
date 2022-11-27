use crate::common::{authentication::Keychain, Backup, ConnectionId};
use std::collections::HashMap;
use std::io;
use tokio::sync::{mpsc, oneshot, RwLock};

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
    pub(crate) shutdown: oneshot::Sender<()>,
    pub(crate) channel_rx: oneshot::Receiver<(mpsc::Sender<T>, mpsc::Receiver<T>)>,
}

impl<T> ConnectionState<T> {
    pub async fn shutdown_and_wait(self) -> io::Result<(mpsc::Sender<T>, mpsc::Receiver<T>)> {
        let _ = self.shutdown(());
        self.channel_rx
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
    }
}
