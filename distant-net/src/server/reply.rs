use crate::{Id, Response};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
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
            hold: Arc::new(AtomicBool::new(true)),
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
    hold: Arc<AtomicBool>,
}

impl<T> QueuedServerReply<T> {
    /// Updates the hold status for the queue
    ///
    /// * If true, all messages are held until the queue is flushed
    /// * If false, messages are sent directly as they come in
    pub fn hold(&self, hold: bool) {
        self.hold.store(hold, Ordering::Relaxed);
    }

    pub async fn send(&self, data: T) -> io::Result<()> {
        if self.hold.load(Ordering::Relaxed) {
            self.queue.lock().await.push(data);
            Ok(())
        } else {
            self.inner.send(data).await
        }
    }

    /// Sends all pending msgs queued up and clears the queue
    pub async fn flush(&self) -> io::Result<()> {
        // TODO: We need to lock access to send, specifically block when checking the
        //       hold status as we want to avoid additional messages being queued
        //       if we flush and want to remove the hold.
        //
        //       E.g. we flush, but in parallel a call to send goes out prior
        //       to the hold getting cleared, and that send call is waiting at
        //       `self.queue.lock().await`, which means when we go to clear the
        //       hold after this, the message would go into the queue and not
        //       be sent, so it would stay in the queue forever
        //
        //       Instead, we want to support flush clearing the hold, and the
        //       hold access check needs to lock to prevent send from going
        //       down the wrong path while we're flushing

        for data in self.queue.lock().await.drain(..) {
            self.inner.send(data).await?;
        }

        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}
