use crate::{Id, Response};
use std::io;
use tokio::sync::mpsc;

/// Utility to send ad-hoc replies from the server back through the connection
pub struct ServerReply<T> {
    pub(crate) origin_id: Id,
    pub(crate) tx: mpsc::Sender<Response<T>>,
}

impl<T> Clone for ServerReply<T> {
    fn clone(&self) -> Self {
        Self {
            origin_id: self.origin_id,
            tx: self.tx.clone(),
        }
    }
}

impl<T> ServerReply<T> {
    pub async fn send(&self, data: T) -> io::Result<()> {
        self.tx
            .send(Response::new(self.origin_id, data))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Connection reply closed"))
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}
