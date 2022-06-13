use crate::{Id, ServerConnection};
use std::collections::HashMap;
use tokio::sync::RwLock;

/// Contains all top-level state for the server
pub struct ServerState<GlobalData, LocalData> {
    /// Mapping of connection ids to their transports
    pub connections: RwLock<HashMap<Id, ServerConnection<LocalData>>>,

    /// Data that exists outside of individual connections
    pub data: GlobalData,
}

impl<GlobalData, LocalData> ServerState<GlobalData, LocalData> {
    pub fn new(data: GlobalData) -> Self {
        Self {
            connections: RwLock::new(HashMap::new()),
            data,
        }
    }
}
