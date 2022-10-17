use super::ConnectionTask;
use crate::common::{authentication::Keychain, Backup, ConnectionId};
use std::collections::HashMap;
use tokio::sync::{oneshot, RwLock};

/// Contains all top-level state for the server
pub struct ServerState {
    /// Mapping of connection ids to their transports
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
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}
