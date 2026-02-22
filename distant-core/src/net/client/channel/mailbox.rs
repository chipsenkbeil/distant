use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::{io, time};

use crate::net::common::{Id, Response, UntypedResponse};

#[derive(Clone, Debug)]
pub struct PostOffice<T> {
    mailboxes: Arc<Mutex<HashMap<Id, mpsc::Sender<T>>>>,
    default_box: Arc<RwLock<Option<mpsc::Sender<T>>>>,
}

impl<T> Default for PostOffice<T>
where
    T: Send + 'static,
{
    /// Creates a new postoffice with a cleanup interval of 60s
    fn default() -> Self {
        Self::new(Duration::from_secs(60))
    }
}

impl<T> PostOffice<T>
where
    T: Send + 'static,
{
    /// Creates a new post office that delivers to mailboxes, cleaning up orphaned mailboxes
    /// waiting `cleanup` time inbetween attempts
    pub fn new(cleanup: Duration) -> Self {
        let mailboxes = Arc::new(Mutex::new(HashMap::new()));
        let mref = Arc::downgrade(&mailboxes);

        // Spawn a task that will clean up orphaned mailboxes every minute
        tokio::spawn(async move {
            while let Some(m) = Weak::upgrade(&mref) {
                m.lock()
                    .await
                    .retain(|_id, tx: &mut mpsc::Sender<T>| !tx.is_closed());

                // NOTE: Must drop the reference before sleeping, otherwise we block
                //       access to the mailbox map elsewhere and deadlock!
                drop(m);

                // Wait a minute before trying again
                time::sleep(cleanup).await;
            }
        });

        Self {
            mailboxes,
            default_box: Arc::new(RwLock::new(None)),
        }
    }

    /// Creates a new mailbox using the given id and buffer size for maximum values that
    /// can be queued in the mailbox
    pub async fn make_mailbox(&self, id: Id, buffer: usize) -> Mailbox<T> {
        let (tx, rx) = mpsc::channel(buffer);
        self.mailboxes.lock().await.insert(id.clone(), tx);

        Mailbox {
            id,
            rx: Box::new(rx),
        }
    }

    /// Delivers some value to appropriate mailbox, returning false if no mailbox is found
    /// for the specified id or if the mailbox is no longer receiving values
    pub async fn deliver(&self, id: &Id, value: T) -> bool {
        if let Some(tx) = self.mailboxes.lock().await.get_mut(id) {
            let success = tx.send(value).await.is_ok();

            // If failed, we want to remove the mailbox sender as it is no longer valid
            if !success {
                self.mailboxes.lock().await.remove(id);
            }

            success
        } else if let Some(tx) = self.default_box.read().await.as_ref() {
            tx.send(value).await.is_ok()
        } else {
            false
        }
    }

    /// Creates a new default mailbox that will be used whenever no mailbox is found to deliver
    /// mail. This will replace any existing default mailbox.
    pub async fn assign_default_mailbox(&self, buffer: usize) -> Mailbox<T> {
        let (tx, rx) = mpsc::channel(buffer);
        *self.default_box.write().await = Some(tx);

        Mailbox {
            id: "".to_string(),
            rx: Box::new(rx),
        }
    }

    /// Removes the default mailbox such that any mail without a matching mailbox will be dropped
    /// instead of being delivered to a default mailbox.
    pub async fn remove_default_mailbox(&self) {
        *self.default_box.write().await = None;
    }

    /// Returns true if the post office is using a default mailbox for all mail that does not map
    /// to another mailbox.
    pub async fn has_default_mailbox(&self) -> bool {
        self.default_box.read().await.is_some()
    }

    /// Cancels delivery to the mailbox with the specified `id`.
    pub async fn cancel(&self, id: &Id) {
        self.mailboxes.lock().await.remove(id);
    }

    /// Cancels delivery to the mailboxes with the specified `id`s.
    pub async fn cancel_many(&self, ids: impl Iterator<Item = &Id>) {
        let mut lock = self.mailboxes.lock().await;
        for id in ids {
            lock.remove(id);
        }
    }

    /// Cancels delivery to all mailboxes.
    pub async fn cancel_all(&self) {
        self.mailboxes.lock().await.clear();
    }
}

impl<T> PostOffice<Response<T>>
where
    T: Send + 'static,
{
    /// Delivers some response to appropriate mailbox, returning false if no mailbox is found
    /// for the response's origin or if the mailbox is no longer receiving values
    pub async fn deliver_response(&self, res: Response<T>) -> bool {
        self.deliver(&res.origin_id.clone(), res).await
    }
}

impl PostOffice<UntypedResponse<'static>> {
    /// Delivers some response to appropriate mailbox, returning false if no mailbox is found
    /// for the response's origin or if the mailbox is no longer receiving values
    pub async fn deliver_untyped_response(&self, res: UntypedResponse<'static>) -> bool {
        self.deliver(&res.origin_id.clone().into_owned(), res).await
    }
}

/// Error encountered when invoking [`try_recv`] for [`MailboxReceiver`].
pub enum MailboxTryNextError {
    Empty,
    Closed,
}

trait MailboxReceiver: Send + Sync {
    type Output;

    fn try_recv(&mut self) -> Result<Self::Output, MailboxTryNextError>;

    fn recv<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Self::Output>> + Send + 'a>>;

    fn close(&mut self);
}

impl<T: Send> MailboxReceiver for mpsc::Receiver<T> {
    type Output = T;

    fn try_recv(&mut self) -> Result<Self::Output, MailboxTryNextError> {
        match mpsc::Receiver::try_recv(self) {
            Ok(x) => Ok(x),
            Err(mpsc::error::TryRecvError::Empty) => Err(MailboxTryNextError::Empty),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(MailboxTryNextError::Closed),
        }
    }

    fn recv<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Self::Output>> + Send + 'a>>
    {
        Box::pin(async move { mpsc::Receiver::recv(self).await })
    }

    fn close(&mut self) {
        mpsc::Receiver::close(self)
    }
}

struct MappedMailboxReceiver<T, U> {
    rx: Box<dyn MailboxReceiver<Output = T>>,
    f: Box<dyn Fn(T) -> U + Send + Sync>,
}

impl<T: Send, U: Send> MailboxReceiver for MappedMailboxReceiver<T, U> {
    type Output = U;

    fn try_recv(&mut self) -> Result<Self::Output, MailboxTryNextError> {
        match self.rx.try_recv() {
            Ok(x) => Ok((self.f)(x)),
            Err(x) => Err(x),
        }
    }

    fn recv<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Self::Output>> + Send + 'a>>
    {
        Box::pin(async move {
            let value = self.rx.recv().await?;
            Some((self.f)(value))
        })
    }

    fn close(&mut self) {
        self.rx.close()
    }
}

struct MappedOptMailboxReceiver<T, U> {
    rx: Box<dyn MailboxReceiver<Output = T>>,
    f: Box<dyn Fn(T) -> Option<U> + Send + Sync>,
}

impl<T: Send, U: Send> MailboxReceiver for MappedOptMailboxReceiver<T, U> {
    type Output = U;

    fn try_recv(&mut self) -> Result<Self::Output, MailboxTryNextError> {
        match self.rx.try_recv() {
            Ok(x) => match (self.f)(x) {
                Some(x) => Ok(x),
                None => Err(MailboxTryNextError::Empty),
            },
            Err(x) => Err(x),
        }
    }

    fn recv<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Self::Output>> + Send + 'a>>
    {
        Box::pin(async move {
            // Continually receive a new value and convert it to Option<U>
            // until Option<U> == Some(U) or we receive None from our inner receiver
            loop {
                let value = self.rx.recv().await?;
                if let Some(x) = (self.f)(value) {
                    return Some(x);
                }
            }
        })
    }

    fn close(&mut self) {
        self.rx.close()
    }
}

/// Represents a destination for responses
pub struct Mailbox<T> {
    /// Represents id associated with the mailbox
    id: Id,

    /// Underlying mailbox storage
    rx: Box<dyn MailboxReceiver<Output = T>>,
}

impl<T> Mailbox<T> {
    /// Represents id associated with the mailbox
    pub fn id(&self) -> &Id {
        &self.id
    }

    /// Tries to receive the next value in mailbox without blocking or waiting async
    pub fn try_next(&mut self) -> Result<T, MailboxTryNextError> {
        self.rx.try_recv()
    }

    /// Receives next value in mailbox
    pub async fn next(&mut self) -> Option<T> {
        self.rx.recv().await
    }

    /// Receives next value in mailbox, waiting up to duration before timing out
    pub async fn next_timeout(&mut self, duration: Duration) -> io::Result<Option<T>> {
        time::timeout(duration, self.next())
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
    }

    /// Closes the mailbox such that it will not receive any more values
    ///
    /// Any values already in the mailbox will still be returned via `next`
    pub fn close(&mut self) {
        self.rx.close()
    }
}

impl<T: Send + 'static> Mailbox<T> {
    /// Maps the results of each mailbox value into a new type `U`
    pub fn map<U: Send + 'static>(self, f: impl Fn(T) -> U + Send + Sync + 'static) -> Mailbox<U> {
        Mailbox {
            id: self.id,
            rx: Box::new(MappedMailboxReceiver {
                rx: self.rx,
                f: Box::new(f),
            }),
        }
    }

    /// Maps the results of each mailbox value into a new type `U` by returning an `Option<U>`
    /// where the option is `None` in the case that `T` cannot be converted into `U`
    pub fn map_opt<U: Send + 'static>(
        self,
        f: impl Fn(T) -> Option<U> + Send + Sync + 'static,
    ) -> Mailbox<U> {
        Mailbox {
            id: self.id,
            rx: Box::new(MappedOptMailboxReceiver {
                rx: self.rx,
                f: Box::new(f),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use test_log::test;

    use super::*;
    use crate::net::common::{Response, UntypedResponse};

    /// Helper to unwrap a try_next result without requiring Debug on MailboxTryNextError.
    fn unwrap_try_next<T: std::fmt::Debug>(result: Result<T, MailboxTryNextError>) -> T {
        match result {
            Ok(v) => v,
            Err(MailboxTryNextError::Empty) => panic!("Expected Ok, got Err(Empty)"),
            Err(MailboxTryNextError::Closed) => panic!("Expected Ok, got Err(Closed)"),
        }
    }

    // ---------------------------------------------------------------
    // PostOffice tests
    // ---------------------------------------------------------------

    #[test(tokio::test)]
    async fn post_office_make_mailbox_and_deliver_round_trip() {
        let po = PostOffice::<u32>::default();
        let mut mb = po.make_mailbox("id1".to_string(), 4).await;

        assert!(po.deliver(&"id1".to_string(), 42).await);
        assert_eq!(mb.next().await, Some(42));
    }

    #[test(tokio::test)]
    async fn post_office_deliver_returns_false_for_unknown_id() {
        let po = PostOffice::<u32>::default();
        assert!(!po.deliver(&"unknown".to_string(), 1).await);
    }

    #[test(tokio::test)]
    async fn post_office_deliver_returns_false_after_mailbox_cancelled() {
        let po = PostOffice::<u32>::default();
        let _mb = po.make_mailbox("id1".to_string(), 4).await;

        // Cancel the mailbox (removes the sender from the map)
        po.cancel(&"id1".to_string()).await;

        // Now delivery should return false since the mailbox is gone
        assert!(!po.deliver(&"id1".to_string(), 99).await);
    }

    #[test(tokio::test)]
    async fn post_office_assign_default_mailbox_catches_undelivered() {
        let po = PostOffice::<u32>::default();
        let mut default_mb = po.assign_default_mailbox(4).await;

        // Send to a non-existent mailbox — should go to default
        assert!(po.deliver(&"nonexistent".to_string(), 7).await);
        assert_eq!(default_mb.next().await, Some(7));
    }

    #[test(tokio::test)]
    async fn post_office_remove_default_mailbox_stops_catching() {
        let po = PostOffice::<u32>::default();
        let _default_mb = po.assign_default_mailbox(4).await;
        po.remove_default_mailbox().await;

        // Without a default, delivery to unknown id should return false
        assert!(!po.deliver(&"nonexistent".to_string(), 7).await);
    }

    #[test(tokio::test)]
    async fn post_office_has_default_mailbox_returns_correct_value() {
        let po = PostOffice::<u32>::default();
        assert!(!po.has_default_mailbox().await);

        let _mb = po.assign_default_mailbox(4).await;
        assert!(po.has_default_mailbox().await);

        po.remove_default_mailbox().await;
        assert!(!po.has_default_mailbox().await);
    }

    #[test(tokio::test)]
    async fn post_office_cancel_removes_specific_mailbox() {
        let po = PostOffice::<u32>::default();
        let _mb1 = po.make_mailbox("a".to_string(), 4).await;
        let _mb2 = po.make_mailbox("b".to_string(), 4).await;

        po.cancel(&"a".to_string()).await;

        assert!(!po.deliver(&"a".to_string(), 1).await);
        assert!(po.deliver(&"b".to_string(), 2).await);
    }

    #[test(tokio::test)]
    async fn post_office_cancel_many_removes_multiple() {
        let po = PostOffice::<u32>::default();
        let _mb1 = po.make_mailbox("x".to_string(), 4).await;
        let _mb2 = po.make_mailbox("y".to_string(), 4).await;
        let _mb3 = po.make_mailbox("z".to_string(), 4).await;

        let ids = vec!["x".to_string(), "y".to_string()];
        po.cancel_many(ids.iter()).await;

        assert!(!po.deliver(&"x".to_string(), 1).await);
        assert!(!po.deliver(&"y".to_string(), 2).await);
        assert!(po.deliver(&"z".to_string(), 3).await);
    }

    #[test(tokio::test)]
    async fn post_office_cancel_all_removes_all() {
        let po = PostOffice::<u32>::default();
        let _mb1 = po.make_mailbox("a".to_string(), 4).await;
        let _mb2 = po.make_mailbox("b".to_string(), 4).await;

        po.cancel_all().await;

        assert!(!po.deliver(&"a".to_string(), 1).await);
        assert!(!po.deliver(&"b".to_string(), 2).await);
    }

    #[test(tokio::test)]
    async fn post_office_deliver_response_uses_origin_id() {
        let po = PostOffice::<Response<u8>>::default();
        let origin = "origin123".to_string();
        let mut mb = po.make_mailbox(origin.clone(), 4).await;

        let res = Response::new(origin.clone(), 42u8);
        assert!(po.deliver_response(res.clone()).await);
        assert_eq!(mb.next().await.unwrap().payload, 42);
    }

    #[test(tokio::test)]
    async fn post_office_deliver_untyped_response_uses_origin_id() {
        let po = PostOffice::<UntypedResponse<'static>>::default();
        let origin = "origin456".to_string();
        let mut mb = po.make_mailbox(origin.clone(), 4).await;

        let res = UntypedResponse {
            header: Cow::Owned(vec![]),
            id: Cow::Owned("resp1".to_string()),
            origin_id: Cow::Owned(origin.clone()),
            payload: Cow::Owned(vec![0xc3]),
        };
        assert!(po.deliver_untyped_response(res.clone()).await);
        let received = mb.next().await.unwrap();
        assert_eq!(received.origin_id.as_ref(), origin.as_str());
    }

    // ---------------------------------------------------------------
    // Mailbox tests
    // ---------------------------------------------------------------

    #[test(tokio::test)]
    async fn mailbox_id_returns_assigned_id() {
        let po = PostOffice::<u32>::default();
        let mb = po.make_mailbox("test_id".to_string(), 4).await;
        assert_eq!(mb.id(), "test_id");
    }

    #[test(tokio::test)]
    async fn mailbox_try_next_returns_empty_when_nothing_queued() {
        let po = PostOffice::<u32>::default();
        let mut mb = po.make_mailbox("id1".to_string(), 4).await;
        match mb.try_next() {
            Err(MailboxTryNextError::Empty) => {}
            other => panic!("Expected Empty, got {:?}", other.ok()),
        }
    }

    #[test(tokio::test)]
    async fn mailbox_try_next_returns_value_when_available() {
        let po = PostOffice::<u32>::default();
        let mut mb = po.make_mailbox("id1".to_string(), 4).await;
        po.deliver(&"id1".to_string(), 99).await;
        assert_eq!(unwrap_try_next(mb.try_next()), 99);
    }

    #[test(tokio::test)]
    async fn mailbox_try_next_returns_closed_after_sender_dropped() {
        let (tx, rx) = mpsc::channel::<u32>(4);
        drop(tx);
        let mut mb = Mailbox {
            id: "closed".to_string(),
            rx: Box::new(rx),
        };
        match mb.try_next() {
            Err(MailboxTryNextError::Closed) => {}
            other => panic!("Expected Closed, got {:?}", other.ok()),
        }
    }

    #[test(tokio::test)]
    async fn mailbox_next_returns_some_when_value_delivered() {
        let po = PostOffice::<u32>::default();
        let mut mb = po.make_mailbox("id1".to_string(), 4).await;

        // Deliver in a separate task so we do not deadlock on next()
        let po2 = po.clone();
        tokio::spawn(async move {
            po2.deliver(&"id1".to_string(), 55).await;
        });

        assert_eq!(mb.next().await, Some(55));
    }

    #[test(tokio::test)]
    async fn mailbox_next_returns_none_after_cancel() {
        let po = PostOffice::<u32>::default();
        let mut mb = po.make_mailbox("id1".to_string(), 4).await;

        let po2 = po.clone();
        tokio::spawn(async move {
            po2.cancel(&"id1".to_string()).await;
        });

        assert_eq!(mb.next().await, None);
    }

    #[test(tokio::test)]
    async fn mailbox_next_timeout_returns_error_on_timeout() {
        let po = PostOffice::<u32>::default();
        let mut mb = po.make_mailbox("id1".to_string(), 4).await;

        let result = mb.next_timeout(Duration::from_millis(10)).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    }

    #[test(tokio::test)]
    async fn mailbox_next_timeout_returns_value_before_timeout() {
        let po = PostOffice::<u32>::default();
        let mut mb = po.make_mailbox("id1".to_string(), 4).await;
        po.deliver(&"id1".to_string(), 77).await;

        let result = mb.next_timeout(Duration::from_secs(1)).await;
        assert_eq!(result.unwrap(), Some(77));
    }

    #[test(tokio::test)]
    async fn mailbox_close_allows_draining_existing_values() {
        let po = PostOffice::<u32>::default();
        let mut mb = po.make_mailbox("id1".to_string(), 4).await;

        // Deliver one value before close
        po.deliver(&"id1".to_string(), 10).await;
        mb.close();

        // Already-queued value is still available after close
        assert_eq!(unwrap_try_next(mb.try_next()), 10);

        // After close + drain, try_next should report closed
        match mb.try_next() {
            Err(MailboxTryNextError::Closed) => {}
            other => panic!("Expected Closed after drain, got {:?}", other.ok()),
        }
    }

    #[test(tokio::test)]
    async fn mailbox_map_transforms_values() {
        let po = PostOffice::<u32>::default();
        let mb = po.make_mailbox("id1".to_string(), 4).await;

        // map u32 -> String
        let mut mapped = mb.map(|v| format!("val={v}"));
        assert_eq!(mapped.id(), "id1");

        po.deliver(&"id1".to_string(), 3).await;
        assert_eq!(mapped.next().await, Some("val=3".to_string()));
    }

    #[test(tokio::test)]
    async fn mailbox_map_opt_filters_and_transforms() {
        let po = PostOffice::<u32>::default();
        let mb = po.make_mailbox("id1".to_string(), 4).await;

        // Only keep even values, converted to strings
        let mut mapped = mb.map_opt(|v| if v % 2 == 0 { Some(v * 10) } else { None });
        assert_eq!(mapped.id(), "id1");

        // Deliver odd (filtered out), then even (kept)
        po.deliver(&"id1".to_string(), 1).await;
        po.deliver(&"id1".to_string(), 4).await;

        // We should get the even value (4*10 = 40), the odd one is skipped
        assert_eq!(mapped.next().await, Some(40));
    }

    #[test(tokio::test)]
    async fn mailbox_map_opt_try_next_returns_empty_when_filtered() {
        let po = PostOffice::<u32>::default();
        let mb = po.make_mailbox("id1".to_string(), 4).await;
        let mut mapped = mb.map_opt(|v| if v > 100 { Some(v) } else { None });

        // Deliver a value that will be filtered
        po.deliver(&"id1".to_string(), 5).await;

        // try_next should see the value but filter it and return Empty
        match mapped.try_next() {
            Err(MailboxTryNextError::Empty) => {}
            other => panic!("Expected Empty, got {:?}", other.ok()),
        }
    }

    #[test(tokio::test)]
    async fn post_office_deliver_multiple_values_to_same_mailbox() {
        let po = PostOffice::<u32>::default();
        let mut mb = po.make_mailbox("id1".to_string(), 10).await;

        for i in 0..5 {
            assert!(po.deliver(&"id1".to_string(), i).await);
        }

        for i in 0..5 {
            assert_eq!(unwrap_try_next(mb.try_next()), i);
        }
    }

    #[test(tokio::test)]
    async fn post_office_default_mailbox_replaced_on_reassign() {
        let po = PostOffice::<u32>::default();
        let _mb1 = po.assign_default_mailbox(4).await;
        let mut mb2 = po.assign_default_mailbox(4).await;

        // Deliver to non-existent id — should go to the new default
        po.deliver(&"nope".to_string(), 123).await;
        assert_eq!(mb2.next().await, Some(123));
    }

    #[test(tokio::test)]
    async fn post_office_specific_mailbox_takes_priority_over_default() {
        let po = PostOffice::<u32>::default();
        let mut specific = po.make_mailbox("specific".to_string(), 4).await;
        let mut default = po.assign_default_mailbox(4).await;

        // Deliver to the specific id — should NOT go to default
        assert!(po.deliver(&"specific".to_string(), 1).await);
        assert_eq!(unwrap_try_next(specific.try_next()), 1);

        // Nothing should be in the default
        match default.try_next() {
            Err(MailboxTryNextError::Empty) => {}
            other => panic!("Expected Empty from default, got {:?}", other.ok()),
        }

        // Deliver to unknown id — should go to default
        assert!(po.deliver(&"unknown".to_string(), 2).await);
        assert_eq!(unwrap_try_next(default.try_next()), 2);
    }
}
