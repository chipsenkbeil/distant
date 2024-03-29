use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::{io, time};

use crate::common::{Id, Response, UntypedResponse};

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

#[async_trait]
trait MailboxReceiver: Send + Sync {
    type Output;

    fn try_recv(&mut self) -> Result<Self::Output, MailboxTryNextError>;

    async fn recv(&mut self) -> Option<Self::Output>;

    fn close(&mut self);
}

#[async_trait]
impl<T: Send> MailboxReceiver for mpsc::Receiver<T> {
    type Output = T;

    fn try_recv(&mut self) -> Result<Self::Output, MailboxTryNextError> {
        match mpsc::Receiver::try_recv(self) {
            Ok(x) => Ok(x),
            Err(mpsc::error::TryRecvError::Empty) => Err(MailboxTryNextError::Empty),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(MailboxTryNextError::Closed),
        }
    }

    async fn recv(&mut self) -> Option<Self::Output> {
        mpsc::Receiver::recv(self).await
    }

    fn close(&mut self) {
        mpsc::Receiver::close(self)
    }
}

struct MappedMailboxReceiver<T, U> {
    rx: Box<dyn MailboxReceiver<Output = T>>,
    f: Box<dyn Fn(T) -> U + Send + Sync>,
}

#[async_trait]
impl<T: Send, U: Send> MailboxReceiver for MappedMailboxReceiver<T, U> {
    type Output = U;

    fn try_recv(&mut self) -> Result<Self::Output, MailboxTryNextError> {
        match self.rx.try_recv() {
            Ok(x) => Ok((self.f)(x)),
            Err(x) => Err(x),
        }
    }

    async fn recv(&mut self) -> Option<Self::Output> {
        let value = self.rx.recv().await?;
        Some((self.f)(value))
    }

    fn close(&mut self) {
        self.rx.close()
    }
}

struct MappedOptMailboxReceiver<T, U> {
    rx: Box<dyn MailboxReceiver<Output = T>>,
    f: Box<dyn Fn(T) -> Option<U> + Send + Sync>,
}

#[async_trait]
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

    async fn recv(&mut self) -> Option<Self::Output> {
        // Continually receive a new value and convert it to Option<U>
        // until Option<U> == Some(U) or we receive None from our inner receiver
        loop {
            let value = self.rx.recv().await?;
            if let Some(x) = (self.f)(value) {
                return Some(x);
            }
        }
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
