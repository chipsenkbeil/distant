use crate::{Id, ServerConnection};
use std::collections::HashMap;
use tokio::sync::RwLock;

/// Contains all top-level state for the server
pub struct ServerState<T> {
    /// Mapping of connection ids to their transports
    pub connections: RwLock<HashMap<Id, ServerConnection<T>>>,
}

impl<T> ServerState<T> {
    pub fn new() -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
        }
    }
}

impl<T> Default for ServerState<T> {
    fn default() -> Self {
        Self::new()
    }
}
