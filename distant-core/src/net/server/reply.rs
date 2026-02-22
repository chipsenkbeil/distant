use std::io;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::net::common::{Id, Response};

/// Interface to send a reply to some request
pub trait Reply: Send + Sync {
    type Data;

    /// Sends a reply out from the server.
    fn send(&self, data: Self::Data) -> io::Result<()>;

    /// Clones this reply.
    fn clone_reply(&self) -> Box<dyn Reply<Data = Self::Data>>;
}

impl<T: Send + 'static> Reply for mpsc::UnboundedSender<T> {
    type Data = T;

    fn send(&self, data: Self::Data) -> io::Result<()> {
        mpsc::UnboundedSender::send(self, data).map_err(|x| io::Error::other(x.to_string()))
    }

    fn clone_reply(&self) -> Box<dyn Reply<Data = Self::Data>> {
        Box::new(self.clone())
    }
}

/// Utility to send ad-hoc replies from the server back through the connection
pub struct ServerReply<T> {
    pub(crate) origin_id: Id,
    pub(crate) tx: mpsc::UnboundedSender<Response<T>>,
}

impl<T> Clone for ServerReply<T> {
    fn clone(&self) -> Self {
        Self {
            origin_id: self.origin_id.clone(),
            tx: self.tx.clone(),
        }
    }
}

impl<T> ServerReply<T> {
    pub fn send(&self, data: T) -> io::Result<()> {
        self.tx
            .send(Response::new(self.origin_id.clone(), data))
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

impl<T: Send + 'static> Reply for ServerReply<T> {
    type Data = T;

    fn send(&self, data: Self::Data) -> io::Result<()> {
        ServerReply::send(self, data)
    }

    fn clone_reply(&self) -> Box<dyn Reply<Data = Self::Data>> {
        Box::new(self.clone())
    }
}

/// Represents a reply where all sends are queued up but not sent until
/// after the flush method is called. This reply supports injecting
/// at the front of the queue in order to support sending messages
/// but ensuring that some specific message is sent out first
pub struct QueuedServerReply<T> {
    inner: ServerReply<T>,
    queue: Arc<Mutex<Vec<T>>>,
    hold: Arc<Mutex<bool>>,
}

impl<T> Clone for QueuedServerReply<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            queue: Arc::clone(&self.queue),
            hold: Arc::clone(&self.hold),
        }
    }
}

impl<T> QueuedServerReply<T> {
    /// Updates the hold status for the queue
    ///
    /// * If true, all messages are held until the queue is flushed
    /// * If false, messages are sent directly as they come in
    pub fn hold(&self, hold: bool) {
        *self.hold.lock().unwrap() = hold;
    }

    /// Send this message, adding it to a queue if holding messages.
    pub fn send(&self, data: T) -> io::Result<()> {
        if *self.hold.lock().unwrap() {
            self.queue.lock().unwrap().push(data);
            Ok(())
        } else {
            self.inner.send(data)
        }
    }

    /// Send this message before anything else in the queue
    pub fn send_before(&self, data: T) -> io::Result<()> {
        if *self.hold.lock().unwrap() {
            self.queue.lock().unwrap().insert(0, data);
            Ok(())
        } else {
            self.inner.send(data)
        }
    }

    /// Sends all pending msgs queued up and clears the queue
    ///
    /// Additionally, takes `hold` to indicate whether or not new msgs
    /// after the flush should continue to be held within the queue
    /// or if all future msgs will be sent immediately
    pub fn flush(&self, hold: bool) -> io::Result<()> {
        // Lock hold so we can ensure that nothing gets sent
        // to the queue after we clear it
        let mut hold_lock = self.hold.lock().unwrap();

        // Clear the queue by sending everything
        for data in self.queue.lock().unwrap().drain(..) {
            self.inner.send(data)?;
        }

        // Update hold to
        *hold_lock = hold;

        Ok(())
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

impl<T: Send + 'static> Reply for QueuedServerReply<T> {
    type Data = T;

    fn send(&self, data: Self::Data) -> io::Result<()> {
        QueuedServerReply::send(self, data)
    }

    fn clone_reply(&self) -> Box<dyn Reply<Data = Self::Data>> {
        Box::new(self.clone())
    }
}
