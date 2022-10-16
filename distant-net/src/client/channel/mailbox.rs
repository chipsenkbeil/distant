use crate::common::{Id, Response};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{
    io,
    sync::{mpsc, Mutex},
    time,
};

#[derive(Clone)]
pub struct PostOffice<T> {
    mailboxes: Arc<Mutex<HashMap<Id, mpsc::Sender<T>>>>,
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

        Self { mailboxes }
    }

    /// Creates a new mailbox using the given id and buffer size for maximum values that
    /// can be queued in the mailbox
    pub async fn make_mailbox(&self, id: Id, buffer: usize) -> Mailbox<T> {
        let (tx, rx) = mpsc::channel(buffer);
        self.mailboxes.lock().await.insert(id.clone(), tx);

        Mailbox { id, rx }
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
        } else {
            false
        }
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

/// Represents a destination for responses
pub struct Mailbox<T> {
    /// Represents id associated with the mailbox
    id: Id,

    /// Underlying mailbox storage
    rx: mpsc::Receiver<T>,
}

impl<T> Mailbox<T> {
    /// Represents id associated with the mailbox
    pub fn id(&self) -> &Id {
        &self.id
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
