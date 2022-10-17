use crate::common::{Request, Response};
use std::{convert, io, sync::Weak};
use tokio::{sync::mpsc, time::Duration};

mod mailbox;
pub use mailbox::*;

/// Capacity associated with a channel's mailboxes for receiving multiple responses to a request
const CHANNEL_MAILBOX_CAPACITY: usize = 10000;

/// Represents a sender of requests tied to a session, holding onto a weak reference of
/// mailboxes to relay responses, meaning that once the [`Session`] is closed or dropped,
/// any sent request will no longer be able to receive responses
pub struct Channel<T, U> {
    /// Used to send requests to a server
    pub(crate) tx: mpsc::Sender<Request<T>>,

    /// Collection of mailboxes for receiving responses to requests
    pub(crate) post_office: Weak<PostOffice<Response<U>>>,
}

// NOTE: Implemented manually to avoid needing clone to be defined on generic types
impl<T, U> Clone for Channel<T, U> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            post_office: Weak::clone(&self.post_office),
        }
    }
}

impl<T, U> Channel<T, U>
where
    T: Send + Sync,
    U: Send + Sync + 'static,
{
    /// Returns true if no more requests can be transferred
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Sends a request and returns a mailbox that can receive one or more responses, failing if
    /// unable to send a request or if the session's receiving line to the remote server has
    /// already been severed
    pub async fn mail(&mut self, req: impl Into<Request<T>>) -> io::Result<Mailbox<Response<U>>> {
        let req = req.into();

        // First, create a mailbox using the request's id
        let mailbox = Weak::upgrade(&self.post_office)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "Session's post office is no longer available",
                )
            })?
            .make_mailbox(req.id.clone(), CHANNEL_MAILBOX_CAPACITY)
            .await;

        // Second, send the request
        self.fire(req).await?;

        // Third, return mailbox
        Ok(mailbox)
    }

    /// Sends a request and returns a mailbox, timing out after duration has passed
    pub async fn mail_timeout(
        &mut self,
        req: impl Into<Request<T>>,
        duration: impl Into<Option<Duration>>,
    ) -> io::Result<Mailbox<Response<U>>> {
        match duration.into() {
            Some(duration) => tokio::time::timeout(duration, self.mail(req))
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
                .and_then(convert::identity),
            None => self.mail(req).await,
        }
    }

    /// Sends a request and waits for a response, failing if unable to send a request or if
    /// the session's receiving line to the remote server has already been severed
    pub async fn send(&mut self, req: impl Into<Request<T>>) -> io::Result<Response<U>> {
        // Send mail and get back a mailbox
        let mut mailbox = self.mail(req).await?;

        // Wait for first response, and then drop the mailbox
        mailbox
            .next()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::ConnectionAborted))
    }

    /// Sends a request and waits for a response, timing out after duration has passed
    pub async fn send_timeout(
        &mut self,
        req: impl Into<Request<T>>,
        duration: impl Into<Option<Duration>>,
    ) -> io::Result<Response<U>> {
        match duration.into() {
            Some(duration) => tokio::time::timeout(duration, self.send(req))
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
                .and_then(convert::identity),
            None => self.send(req).await,
        }
    }

    /// Sends a request without waiting for a response; this method is able to be used even
    /// if the session's receiving line to the remote server has been severed
    pub async fn fire(&mut self, req: impl Into<Request<T>>) -> io::Result<()> {
        self.tx
            .send(req.into())
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x.to_string()))
    }

    /// Sends a request without waiting for a response, timing out after duration has passed
    pub async fn fire_timeout(
        &mut self,
        req: impl Into<Request<T>>,
        duration: impl Into<Option<Duration>>,
    ) -> io::Result<()> {
        match duration.into() {
            Some(duration) => tokio::time::timeout(duration, self.fire(req))
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
                .and_then(convert::identity),
            None => self.fire(req).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use test_log::test;

    type TestChannel = Channel<u8, u8>;
    type Setup = (
        TestChannel,
        mpsc::Receiver<Request<u8>>,
        Arc<PostOffice<Response<u8>>>,
    );

    fn setup(buffer: usize) -> Setup {
        let post_office = Arc::new(PostOffice::default());
        let (tx, rx) = mpsc::channel(buffer);
        let channel = {
            let post_office = Arc::downgrade(&post_office);
            TestChannel { tx, post_office }
        };

        (channel, rx, post_office)
    }

    #[test(tokio::test)]
    async fn mail_should_return_mailbox_that_receives_responses_until_post_office_drops_it() {
        let (mut channel, _server, post_office) = setup(100);

        let req = Request::new(0);
        let res = Response::new(req.id.clone(), 1);

        let mut mailbox = channel.mail(req).await.unwrap();

        // Send and receive first response
        assert!(
            post_office.deliver_response(res.clone()).await,
            "Failed to deliver: {res:?}"
        );
        assert_eq!(mailbox.next().await, Some(res.clone()));

        // Send and receive second response
        assert!(
            post_office.deliver_response(res.clone()).await,
            "Failed to deliver: {res:?}"
        );
        assert_eq!(mailbox.next().await, Some(res.clone()));

        // Trigger the mailbox to wait BEFORE closing our mailbox to ensure that
        // we don't get stuck if the mailbox was already waiting
        let next_task = tokio::spawn(async move { mailbox.next().await });
        tokio::task::yield_now().await;

        // Close our specific mailbox
        post_office.cancel(&res.origin_id).await;

        match next_task.await {
            Ok(None) => {}
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn send_should_wait_until_response_received() {
        let (mut channel, _server, post_office) = setup(100);

        let req = Request::new(0);
        let res = Response::new(req.id.clone(), 1);

        let (actual, _) =
            tokio::join!(channel.send(req), post_office.deliver_response(res.clone()));
        match actual {
            Ok(actual) => assert_eq!(actual, res),
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn send_timeout_should_fail_if_response_not_received_in_time() {
        let (mut channel, mut server, _post_office) = setup(100);

        let req = Request::new(0);
        match channel.send_timeout(req, Duration::from_millis(30)).await {
            Err(x) => assert_eq!(x.kind(), io::ErrorKind::TimedOut),
            x => panic!("Unexpected response: {:?}", x),
        }

        let _frame = server.recv().await.unwrap();
    }

    #[test(tokio::test)]
    async fn fire_should_send_request_and_not_wait_for_response() {
        let (mut channel, mut server, _post_office) = setup(100);

        let req = Request::new(0);
        match channel.fire(req).await {
            Ok(_) => {}
            x => panic!("Unexpected response: {:?}", x),
        }

        let _frame = server.recv().await.unwrap();
    }
}
