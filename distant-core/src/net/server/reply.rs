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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::common::Response;
    use tokio::sync::mpsc;

    // ---- UnboundedSender Reply impl ----

    #[test_log::test(tokio::test)]
    async fn unbounded_sender_reply_send_should_succeed() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        Reply::send(&tx, "hello".to_string()).unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received, "hello");
    }

    #[test_log::test(tokio::test)]
    async fn unbounded_sender_reply_send_should_fail_when_receiver_dropped() {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        drop(rx);
        let result = Reply::send(&tx, "hello".to_string());
        assert!(result.is_err());
    }

    #[test_log::test(tokio::test)]
    async fn unbounded_sender_clone_reply_returns_working_sender() {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let cloned = Reply::clone_reply(&tx);
        cloned.send("from clone".to_string()).unwrap();
        let received = rx.recv().await.unwrap();
        assert_eq!(received, "from clone");
    }

    // ---- ServerReply ----

    fn make_server_reply() -> (
        ServerReply<String>,
        mpsc::UnboundedReceiver<Response<String>>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let reply = ServerReply {
            origin_id: "test-origin".to_string(),
            tx,
        };
        (reply, rx)
    }

    #[test_log::test(tokio::test)]
    async fn server_reply_send_creates_response_with_correct_origin_id() {
        let (reply, mut rx) = make_server_reply();
        reply.send("payload".to_string()).unwrap();
        let response = rx.recv().await.unwrap();
        assert_eq!(response.origin_id, "test-origin");
        assert_eq!(response.payload, "payload");
    }

    #[test_log::test(tokio::test)]
    async fn server_reply_send_fails_when_receiver_dropped() {
        let (reply, rx) = make_server_reply();
        drop(rx);
        let result = reply.send("payload".to_string());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test_log::test(tokio::test)]
    async fn server_reply_is_closed_returns_false_when_receiver_alive() {
        let (reply, _rx) = make_server_reply();
        assert!(!reply.is_closed());
    }

    #[test_log::test(tokio::test)]
    async fn server_reply_is_closed_returns_true_when_receiver_dropped() {
        let (reply, rx) = make_server_reply();
        drop(rx);
        assert!(reply.is_closed());
    }

    #[test_log::test(tokio::test)]
    async fn server_reply_clone_shares_tx() {
        let (reply, mut rx) = make_server_reply();
        let cloned = reply.clone();
        cloned.send("from clone".to_string()).unwrap();
        let response = rx.recv().await.unwrap();
        assert_eq!(response.origin_id, "test-origin");
        assert_eq!(response.payload, "from clone");
    }

    #[test_log::test(tokio::test)]
    async fn server_reply_clone_preserves_origin_id() {
        let (reply, _rx) = make_server_reply();
        let cloned = reply.clone();
        assert_eq!(cloned.origin_id, "test-origin");
    }

    #[test_log::test(tokio::test)]
    async fn server_reply_trait_impl_send_works() {
        let (reply, mut rx) = make_server_reply();
        Reply::send(&reply, "via trait".to_string()).unwrap();
        let response = rx.recv().await.unwrap();
        assert_eq!(response.payload, "via trait");
    }

    #[test_log::test(tokio::test)]
    async fn server_reply_trait_impl_clone_reply_works() {
        let (reply, mut rx) = make_server_reply();
        let cloned = Reply::clone_reply(&reply);
        cloned.send("via trait clone".to_string()).unwrap();
        let response = rx.recv().await.unwrap();
        assert_eq!(response.payload, "via trait clone");
    }

    // ---- ServerReply::queue -> QueuedServerReply ----

    #[test_log::test(tokio::test)]
    async fn server_reply_queue_creates_queued_reply_in_hold_mode() {
        let (reply, _rx) = make_server_reply();
        let queued = reply.queue();
        // By default, hold is true so send should queue, not send immediately
        queued.send("queued".to_string()).unwrap();
        // Nothing should be received yet
    }

    // ---- QueuedServerReply ----

    fn make_queued_reply() -> (
        QueuedServerReply<String>,
        mpsc::UnboundedReceiver<Response<String>>,
    ) {
        let (reply, rx) = make_server_reply();
        (reply.queue(), rx)
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_send_while_held_queues_data() {
        let (queued, mut rx) = make_queued_reply();
        queued.send("first".to_string()).unwrap();
        queued.send("second".to_string()).unwrap();

        // Nothing should have been sent through yet
        assert!(rx.try_recv().is_err());
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_send_while_not_held_sends_immediately() {
        let (queued, mut rx) = make_queued_reply();
        queued.hold(false);
        queued.send("immediate".to_string()).unwrap();

        let response = rx.recv().await.unwrap();
        assert_eq!(response.payload, "immediate");
        assert_eq!(response.origin_id, "test-origin");
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_send_before_inserts_at_front_of_queue() {
        let (queued, mut rx) = make_queued_reply();
        queued.send("second".to_string()).unwrap();
        queued.send("third".to_string()).unwrap();
        queued.send_before("first".to_string()).unwrap();

        queued.flush(false).unwrap();

        let r1 = rx.recv().await.unwrap();
        let r2 = rx.recv().await.unwrap();
        let r3 = rx.recv().await.unwrap();
        assert_eq!(r1.payload, "first");
        assert_eq!(r2.payload, "second");
        assert_eq!(r3.payload, "third");
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_send_before_while_not_held_sends_immediately() {
        let (queued, mut rx) = make_queued_reply();
        queued.hold(false);
        queued.send_before("immediate".to_string()).unwrap();

        let response = rx.recv().await.unwrap();
        assert_eq!(response.payload, "immediate");
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_flush_sends_all_queued_and_clears() {
        let (queued, mut rx) = make_queued_reply();
        queued.send("a".to_string()).unwrap();
        queued.send("b".to_string()).unwrap();
        queued.send("c".to_string()).unwrap();

        queued.flush(true).unwrap();

        let r1 = rx.recv().await.unwrap();
        let r2 = rx.recv().await.unwrap();
        let r3 = rx.recv().await.unwrap();
        assert_eq!(r1.payload, "a");
        assert_eq!(r2.payload, "b");
        assert_eq!(r3.payload, "c");

        // Queue should be empty; sending again should queue (hold=true was passed to flush)
        queued.send("d".to_string()).unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_flush_with_hold_false_allows_direct_send_after() {
        let (queued, mut rx) = make_queued_reply();
        queued.send("queued".to_string()).unwrap();

        queued.flush(false).unwrap();

        let r1 = rx.recv().await.unwrap();
        assert_eq!(r1.payload, "queued");

        // After flush with hold=false, new sends go directly
        queued.send("direct".to_string()).unwrap();
        let r2 = rx.recv().await.unwrap();
        assert_eq!(r2.payload, "direct");
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_is_closed_returns_false_when_receiver_alive() {
        let (queued, _rx) = make_queued_reply();
        assert!(!queued.is_closed());
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_is_closed_returns_true_when_receiver_dropped() {
        let (queued, rx) = make_queued_reply();
        drop(rx);
        assert!(queued.is_closed());
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_clone_shares_queue_and_hold() {
        let (queued, mut rx) = make_queued_reply();
        let cloned = queued.clone();

        // Send via clone, should be queued
        cloned.send("from clone".to_string()).unwrap();
        assert!(rx.try_recv().is_err());

        // Flush via original, should send the cloned message too
        queued.flush(false).unwrap();
        let r1 = rx.recv().await.unwrap();
        assert_eq!(r1.payload, "from clone");
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_trait_impl_send_works() {
        let (queued, mut rx) = make_queued_reply();
        queued.hold(false);
        Reply::send(&queued, "via trait".to_string()).unwrap();
        let response = rx.recv().await.unwrap();
        assert_eq!(response.payload, "via trait");
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_trait_impl_clone_reply_works() {
        let (queued, mut rx) = make_queued_reply();
        queued.hold(false);
        let cloned = Reply::clone_reply(&queued);
        cloned.send("via trait clone".to_string()).unwrap();
        let response = rx.recv().await.unwrap();
        assert_eq!(response.payload, "via trait clone");
    }

    #[test_log::test(tokio::test)]
    async fn queued_reply_flush_fails_when_receiver_dropped() {
        let (queued, rx) = make_queued_reply();
        queued.send("data".to_string()).unwrap();
        drop(rx);
        let result = queued.flush(false);
        assert!(result.is_err());
    }
}
