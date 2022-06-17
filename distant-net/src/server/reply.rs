use crate::{Id, Response};
use std::{io, sync::Arc};
use tokio::sync::{mpsc, Mutex};

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

    pub fn queue(self) -> QueuedServerReply<T> {
        QueuedServerReply {
            inner: self,
            queue: Arc::new(Mutex::new(Vec::new())),
            hold: Arc::new(Mutex::new(true)),
        }
    }
}

/// Represents a reply where all sends are queued up but not sent until
/// after the flush method is called. This reply supports injecting
/// at the front of the queue in order to support sending messages
/// but ensuring that some specific message is sent out first
#[derive(Clone)]
pub struct QueuedServerReply<T> {
    inner: ServerReply<T>,
    queue: Arc<Mutex<Vec<T>>>,
    hold: Arc<Mutex<bool>>,
}

impl<T> QueuedServerReply<T> {
    /// Updates the hold status for the queue
    ///
    /// * If true, all messages are held until the queue is flushed
    /// * If false, messages are sent directly as they come in
    pub async fn hold(&self, hold: bool) {
        *self.hold.lock().await = hold;
    }

    /// Send this message, adding it to a queue if holding messages
    pub async fn send(&self, data: T) -> io::Result<()> {
        if *self.hold.lock().await {
            self.queue.lock().await.push(data);
            Ok(())
        } else {
            self.inner.send(data).await
        }
    }

    /// Send this message before anything else in the queue
    pub async fn send_before(&self, data: T) -> io::Result<()> {
        if *self.hold.lock().await {
            self.queue.lock().await.insert(0, data);
            Ok(())
        } else {
            self.inner.send(data).await
        }
    }

    /// Sends all pending msgs queued up and clears the queue
    ///
    /// Additionally, takes `hold` to indicate whether or not new msgs
    /// after the flush should continue to be held within the queue
    /// or if all future msgs will be sent immediately
    pub async fn flush(&self, hold: bool) -> io::Result<()> {
        // Lock hold so we can ensure that nothing gets sent
        // to the queue after we clear it
        let mut hold_lock = self.hold.lock().await;

        // Clear the queue by sending everything
        for data in self.queue.lock().await.drain(..) {
            self.inner.send(data).await?;
        }

        // Update hold to
        *hold_lock = hold;

        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}
