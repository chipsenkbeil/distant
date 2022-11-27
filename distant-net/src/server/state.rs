use super::ConnectionTask;
use crate::common::{authentication::Keychain, Backup, ConnectionId};
use std::collections::HashMap;
use tokio::sync::{oneshot, RwLock};

/// Contains all top-level state for the server
pub struct ServerState {
    /// Mapping of active connection ids to their tasks.
    pub connections: RwLock<HashMap<ConnectionId, ConnectionTask>>,

    /// Mapping of connection ids to (OTP, backup)
    pub keychain: Keychain<oneshot::Receiver<Backup>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            keychain: Keychain::new(),
        }
    }

    /// Returns true if there is at least one active connection.
    pub async fn has_active_connections(&self) -> bool {
        self.connections
            .read()
            .await
            .values()
            .any(|task| !task.is_finished())
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}
