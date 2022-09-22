use crate::{ConnectionId, HeapSecretKey, ServerConnection};
use std::collections::HashMap;
use tokio::sync::RwLock;

/// Contains all top-level state for the server
pub struct ServerState {
    /// Mapping of connection ids to their transports
    pub connections: RwLock<HashMap<ConnectionId, ServerConnection>>,

    /// Mapping of connection ids to their authenticated keys
    pub authenticated: RwLock<HashMap<ConnectionId, HeapSecretKey>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            authenticated: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}
