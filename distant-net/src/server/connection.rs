use crate::Id;
use tokio::task::JoinHandle;

/// Represents an individual connection on the server
pub struct ServerConnection<T> {
    /// Unique identifier tied to the connection
    pub id: Id,

    /// Data associated with the connection
    pub data: T,

    /// Task that is processing incoming requests from the connection
    pub(crate) reader_task: Option<JoinHandle<()>>,

    /// Task that is processing outgoing responses to the connection
    pub(crate) writer_task: Option<JoinHandle<()>>,
}

impl<T: Default> Default for ServerConnection<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> ServerConnection<T> {
    /// Creates a new connection using provided data as default,
    /// generating a unique id to represent the connection
    pub fn new(data: T) -> Self {
        Self {
            id: rand::random(),
            data,
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
