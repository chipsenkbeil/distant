use crate::ConnectionId;
use tokio::task::JoinHandle;

/// Represents an individual connection on the server
pub struct ServerConnection {
    /// Unique identifier tied to the connection
    pub id: ConnectionId,

    /// Task that is processing incoming requests from the connection
    pub(crate) reader_task: Option<JoinHandle<()>>,

    /// Task that is processing outgoing responses to the connection
    pub(crate) writer_task: Option<JoinHandle<()>>,
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
            reader_task: None,
            writer_task: None,
        }
    }

    /// Returns true if connection is still processing incoming or outgoing messages
    pub fn is_active(&self) -> bool {
        let reader_active =
            self.reader_task.is_some() && !self.reader_task.as_ref().unwrap().is_finished();
        let writer_active =
            self.writer_task.is_some() && !self.writer_task.as_ref().unwrap().is_finished();
        reader_active || writer_active
    }

    /// Aborts the connection
    pub fn abort(&self) {
        if let Some(task) = self.reader_task.as_ref() {
            task.abort();
        }

        if let Some(task) = self.writer_task.as_ref() {
            task.abort();
        }
    }
}
