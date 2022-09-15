use crate::ConnectionId;
use tokio::task::JoinHandle;

/// Represents an individual connection on the server
pub struct ServerConnection {
    /// Unique identifier tied to the connection
    pub id: ConnectionId,

    /// Task that is processing requests and responses
    pub(crate) task: Option<JoinHandle<()>>,
}

impl Default for ServerConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerConnection {
    /// Creates a new connection, generating a unique id to represent the connection
    pub fn new() -> Self {
        Self {
            id: rand::random(),
            task: None,
        }
    }

    /// Returns true if connection is still processing incoming or outgoing messages
    pub fn is_active(&self) -> bool {
        self.task.is_some() && !self.task.as_ref().unwrap().is_finished()
    }

    /// Aborts the connection
    pub fn abort(&self) {
        if let Some(task) = self.task.as_ref() {
            task.abort();
        }
    }
}
