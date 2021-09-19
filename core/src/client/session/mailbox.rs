use crate::{client::utils, data::Response};
use std::{collections::HashMap, time::Duration};
use tokio::{io, sync::mpsc};

pub struct PostOffice {
    mailboxes: HashMap<usize, mpsc::Sender<Response>>,
}

impl PostOffice {
    pub fn new() -> Self {
        Self {
            mailboxes: HashMap::new(),
        }
    }

    /// Creates a new mailbox using the given id and buffer size for maximum messages
    pub fn make_mailbox(&mut self, id: usize, buffer: usize) -> Mailbox {
        let (tx, rx) = mpsc::channel(buffer);
        self.mailboxes.insert(id, tx);

        Mailbox { id, rx }
    }

    /// Delivers a response to appropriate mailbox, returning false if no mailbox is found
    /// for the response or if the mailbox is no longer receiving responses
    pub async fn deliver(&mut self, res: Response) -> bool {
        let id = res.origin_id;

        let success = if let Some(tx) = self.mailboxes.get_mut(&id) {
            tx.send(res).await.is_ok()
        } else {
            false
        };

        // If failed, we want to remove the mailbox sender as it is no longer valid
        if !success {
            self.mailboxes.remove(&id);
        }

        success
    }

    /// Removes all mailboxes from post office that are closed
    pub fn prune_mailboxes(&mut self) {
        self.mailboxes.retain(|_, tx| !tx.is_closed())
    }

    /// Closes out all mailboxes by removing the mailboxes delivery trackers internally
    pub fn clear_mailboxes(&mut self) {
        self.mailboxes.clear();
    }
}

pub struct Mailbox {
    /// Represents id associated with the mailbox
    id: usize,

    /// Underlying mailbox storage
    rx: mpsc::Receiver<Response>,
}

impl Mailbox {
    /// Represents id associated with the mailbox
    pub fn id(&self) -> usize {
        self.id
    }

    /// Receives next response in mailbox
    pub async fn next(&mut self) -> Option<Response> {
        self.rx.recv().await
    }

    /// Receives next response in mailbox, waiting up to duration before timing out
    pub async fn next_timeout(&mut self, duration: Duration) -> io::Result<Option<Response>> {
        utils::timeout(duration, self.next()).await
    }

    /// Closes the mailbox such that it will not receive any more responses
    ///
    /// Any responses already in the mailbox will still be returned via `next`
    pub async fn close(&mut self) {
        self.rx.close()
    }
}
