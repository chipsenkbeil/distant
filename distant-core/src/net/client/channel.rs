use std::marker::PhantomData;
use std::sync::Weak;
use std::{convert, fmt, io};

use log::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::time::Duration;

use crate::net::common::{Request, Response, UntypedRequest, UntypedResponse};

mod mailbox;
pub use mailbox::*;

/// Capacity associated with a channel's mailboxes for receiving multiple responses to a request
const CHANNEL_MAILBOX_CAPACITY: usize = 10000;

/// Represents a sender of requests tied to a session, holding onto a weak reference of
/// mailboxes to relay responses, meaning that once the [`Client`] is closed or dropped,
/// any sent request will no longer be able to receive responses.
///
/// [`Client`]: crate::net::client::Client
pub struct Channel<T, U> {
    inner: UntypedChannel,
    _request: PhantomData<T>,
    _response: PhantomData<U>,
}

// NOTE: Implemented manually to avoid needing clone to be defined on generic types
impl<T, U> Clone for Channel<T, U> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _request: self._request,
            _response: self._response,
        }
    }
}

impl<T, U> fmt::Debug for Channel<T, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("tx", &self.inner.tx)
            .field("post_office", &self.inner.post_office)
            .field("_request", &self._request)
            .field("_response", &self._response)
            .finish()
    }
}

impl<T, U> Channel<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Returns true if no more requests can be transferred
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// Consumes this channel, returning an untyped variant
    pub fn into_untyped_channel(self) -> UntypedChannel {
        self.inner
    }

    /// Assigns a default mailbox for any response received that does not match another mailbox.
    pub async fn assign_default_mailbox(&self, buffer: usize) -> io::Result<Mailbox<Response<U>>> {
        Ok(map_to_typed_mailbox(
            self.inner.assign_default_mailbox(buffer).await?,
        ))
    }

    /// Removes the default mailbox used for unmatched responses such that any response without a
    /// matching mailbox will be dropped.
    pub async fn remove_default_mailbox(&self) -> io::Result<()> {
        self.inner.remove_default_mailbox().await
    }

    /// Sends a request and returns a mailbox that can receive one or more responses, failing if
    /// unable to send a request or if the session's receiving line to the remote server has
    /// already been severed
    pub async fn mail(&mut self, req: impl Into<Request<T>>) -> io::Result<Mailbox<Response<U>>> {
        Ok(map_to_typed_mailbox(
            self.inner.mail(req.into().to_untyped_request()?).await?,
        ))
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
        self.inner.fire(req.into().to_untyped_request()?).await
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

fn map_to_typed_mailbox<T: Send + DeserializeOwned + 'static>(
    mailbox: Mailbox<UntypedResponse<'static>>,
) -> Mailbox<Response<T>> {
    mailbox.map_opt(|res| match res.to_typed_response() {
        Ok(res) => Some(res),
        Err(x) => {
            if log::log_enabled!(Level::Trace) {
                trace!(
                    "Invalid response payload: {}",
                    String::from_utf8_lossy(&res.payload)
                );
            }

            error!(
                "Unable to parse response payload into {}: {x}",
                std::any::type_name::<T>()
            );
            None
        }
    })
}

/// Represents a sender of requests tied to a session, holding onto a weak reference of
/// mailboxes to relay responses, meaning that once the [`Client`] is closed or dropped,
/// any sent request will no longer be able to receive responses.
///
/// In contrast to [`Channel`], this implementation is untyped, meaning that the payload of
/// requests and responses are not validated.
///
/// [`Client`]: crate::net::client::Client
#[derive(Debug)]
pub struct UntypedChannel {
    /// Used to send requests to a server
    pub(crate) tx: mpsc::Sender<UntypedRequest<'static>>,

    /// Collection of mailboxes for receiving responses to requests
    pub(crate) post_office: Weak<PostOffice<UntypedResponse<'static>>>,
}

// NOTE: Implemented manually to avoid needing clone to be defined on generic types
impl Clone for UntypedChannel {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            post_office: Weak::clone(&self.post_office),
        }
    }
}

impl UntypedChannel {
    /// Returns true if no more requests can be transferred
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Consumes this channel, returning a typed variant
    pub fn into_typed_channel<T, U>(self) -> Channel<T, U> {
        Channel {
            inner: self,
            _request: PhantomData,
            _response: PhantomData,
        }
    }

    /// Assigns a default mailbox for any response received that does not match another mailbox.
    pub async fn assign_default_mailbox(
        &self,
        buffer: usize,
    ) -> io::Result<Mailbox<UntypedResponse<'static>>> {
        match Weak::upgrade(&self.post_office) {
            Some(post_office) => Ok(post_office.assign_default_mailbox(buffer).await),
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Channel's post office is no longer available",
            )),
        }
    }

    /// Removes the default mailbox used for unmatched responses such that any response without a
    /// matching mailbox will be dropped.
    pub async fn remove_default_mailbox(&self) -> io::Result<()> {
        match Weak::upgrade(&self.post_office) {
            Some(post_office) => {
                post_office.remove_default_mailbox().await;
                Ok(())
            }
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Channel's post office is no longer available",
            )),
        }
    }

    /// Sends a request and returns a mailbox that can receive one or more responses, failing if
    /// unable to send a request or if the session's receiving line to the remote server has
    /// already been severed
    pub async fn mail(
        &mut self,
        req: UntypedRequest<'_>,
    ) -> io::Result<Mailbox<UntypedResponse<'static>>> {
        // First, create a mailbox using the request's id
        let mailbox = Weak::upgrade(&self.post_office)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotConnected,
                    "Channel's post office is no longer available",
                )
            })?
            .make_mailbox(req.id.clone().into_owned(), CHANNEL_MAILBOX_CAPACITY)
            .await;

        // Second, send the request
        self.fire(req).await?;

        // Third, return mailbox
        Ok(mailbox)
    }

    /// Sends a request and returns a mailbox, timing out after duration has passed
    pub async fn mail_timeout(
        &mut self,
        req: UntypedRequest<'_>,
        duration: impl Into<Option<Duration>>,
    ) -> io::Result<Mailbox<UntypedResponse<'static>>> {
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
    pub async fn send(&mut self, req: UntypedRequest<'_>) -> io::Result<UntypedResponse<'static>> {
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
        req: UntypedRequest<'_>,
        duration: impl Into<Option<Duration>>,
    ) -> io::Result<UntypedResponse<'static>> {
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
    pub async fn fire(&mut self, req: UntypedRequest<'_>) -> io::Result<()> {
        self.tx
            .send(req.into_owned())
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x.to_string()))
    }

    /// Sends a request without waiting for a response, timing out after duration has passed
    pub async fn fire_timeout(
        &mut self,
        req: UntypedRequest<'_>,
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
    //! Tests for Channel<T,U> and UntypedChannel: fire/mail/send operations, timeout variants,
    //! concurrent request routing, deserialization skip behavior, and clone independence.

    use super::*;

    mod typed {
        use std::sync::Arc;
        use std::time::Duration;

        use test_log::test;

        use super::*;

        type TestChannel = Channel<u8, u8>;
        type Setup = (
            TestChannel,
            mpsc::Receiver<UntypedRequest<'static>>,
            Arc<PostOffice<UntypedResponse<'static>>>,
        );

        fn setup(buffer: usize) -> Setup {
            let post_office = Arc::new(PostOffice::default());
            let (tx, rx) = mpsc::channel(buffer);
            let channel = {
                let post_office = Arc::downgrade(&post_office);
                UntypedChannel { tx, post_office }
            };

            (channel.into_typed_channel(), rx, post_office)
        }

        #[test(tokio::test)]
        async fn mail_should_return_mailbox_that_receives_responses_until_post_office_drops_it() {
            let (mut channel, _server, post_office) = setup(100);

            let req = Request::new(0);
            let res = Response::new(req.id.clone(), 1);

            let mut mailbox = channel.mail(req).await.unwrap();

            // Send and receive first response
            assert!(
                post_office
                    .deliver_untyped_response(res.to_untyped_response().unwrap().into_owned())
                    .await,
                "Failed to deliver: {res:?}"
            );
            assert_eq!(mailbox.next().await, Some(res.clone()));

            // Send and receive second response
            assert!(
                post_office
                    .deliver_untyped_response(res.to_untyped_response().unwrap().into_owned())
                    .await,
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

            let (actual, _) = tokio::join!(
                channel.send(req),
                post_office
                    .deliver_untyped_response(res.to_untyped_response().unwrap().into_owned())
            );
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

        #[test(tokio::test)]
        async fn is_closed_should_return_false_when_receiver_exists() {
            let (channel, _server, _post_office) = setup(100);
            assert!(!channel.is_closed());
        }

        #[test(tokio::test)]
        async fn is_closed_should_return_true_when_receiver_dropped() {
            let (channel, server, _post_office) = setup(100);
            drop(server);
            assert!(channel.is_closed());
        }

        #[test(tokio::test)]
        async fn into_untyped_channel_should_produce_working_channel() {
            let (channel, mut server, post_office) = setup(100);
            let mut untyped = channel.into_untyped_channel();

            let req = Request::new(0u8).to_untyped_request().unwrap().into_owned();
            let req_id = req.id.clone().into_owned();
            untyped.fire(req).await.unwrap();

            let received = server.recv().await.unwrap();
            assert_eq!(received.id.as_ref(), req_id.as_str());

            // Verify the post office reference is still valid
            let _mb = untyped.assign_default_mailbox(10).await.unwrap();
            assert!(post_office.has_default_mailbox().await);
        }

        #[test(tokio::test)]
        async fn fire_should_fail_with_broken_pipe_when_receiver_dropped() {
            let (mut channel, server, _post_office) = setup(100);
            drop(server);

            let req = Request::new(0);
            let err = channel.fire(req).await.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        }

        #[test(tokio::test)]
        async fn mail_should_fail_when_post_office_dropped() {
            let (mut channel, _server, post_office) = setup(100);
            drop(post_office);

            let req = Request::new(0);
            match channel.mail(req).await {
                Err(err) => assert_eq!(err.kind(), io::ErrorKind::NotConnected),
                Ok(_) => panic!("Expected NotConnected error"),
            }
        }

        #[test(tokio::test)]
        async fn send_should_fail_with_connection_aborted_when_mailbox_cancelled_before_response() {
            let (mut channel, _server, post_office) = setup(100);

            let req = Request::new(0u8);
            let req_id = req.id.clone();

            // Spawn send in a task, then cancel the mailbox before any response arrives
            let po = post_office.clone();
            let cancel_task = tokio::spawn(async move {
                // Give send a moment to register the mailbox
                tokio::task::yield_now().await;
                po.cancel(&req_id).await;
            });

            let result = channel.send(req).await;
            cancel_task.await.unwrap();

            let err = result.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted);
        }

        #[test(tokio::test)]
        async fn assign_default_mailbox_should_fail_when_post_office_dropped() {
            let (channel, _server, post_office) = setup(100);
            drop(post_office);

            match channel.assign_default_mailbox(10).await {
                Err(err) => assert_eq!(err.kind(), io::ErrorKind::NotConnected),
                Ok(_) => panic!("Expected NotConnected error"),
            }
        }

        #[test(tokio::test)]
        async fn remove_default_mailbox_should_fail_when_post_office_dropped() {
            let (channel, _server, post_office) = setup(100);
            drop(post_office);

            let err = channel.remove_default_mailbox().await.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        }

        #[test(tokio::test)]
        async fn mail_timeout_should_fail_if_fire_blocks_past_deadline() {
            // Buffer of 1 means the second fire will block
            let (mut channel, _server, _post_office) = setup(1);

            // Fill the channel buffer so the next fire will block
            let req_fill = Request::new(0u8);
            channel.fire(req_fill).await.unwrap();

            // Now mail_timeout should time out because fire cannot complete
            let req = Request::new(1u8);
            match channel.mail_timeout(req, Duration::from_millis(30)).await {
                Err(err) => assert_eq!(err.kind(), io::ErrorKind::TimedOut),
                Ok(_) => panic!("Expected TimedOut error"),
            }
        }

        #[test(tokio::test)]
        async fn mail_timeout_with_none_duration_should_behave_like_mail() {
            let (mut channel, _server, post_office) = setup(100);

            let req = Request::new(0u8);
            let res = Response::new(req.id.clone(), 1u8);

            let (mailbox_result, _) = tokio::join!(
                channel.mail_timeout(req, None),
                post_office
                    .deliver_untyped_response(res.to_untyped_response().unwrap().into_owned())
            );

            match mailbox_result {
                Ok(mut mailbox) => assert_eq!(mailbox.next().await, Some(res)),
                Err(err) => panic!("Expected Ok, got Err: {err}"),
            }
        }

        #[test(tokio::test)]
        async fn send_timeout_with_none_duration_should_behave_like_send() {
            let (mut channel, _server, post_office) = setup(100);

            let req = Request::new(0u8);
            let res = Response::new(req.id.clone(), 1u8);

            let (actual, _) = tokio::join!(
                channel.send_timeout(req, None),
                post_office
                    .deliver_untyped_response(res.to_untyped_response().unwrap().into_owned())
            );
            assert_eq!(actual.unwrap(), res);
        }

        #[test(tokio::test)]
        async fn fire_timeout_should_succeed_when_within_time() {
            let (mut channel, mut server, _post_office) = setup(100);

            let req = Request::new(0);
            channel
                .fire_timeout(req, Duration::from_secs(1))
                .await
                .unwrap();

            let _frame = server.recv().await.unwrap();
        }

        #[test(tokio::test)]
        async fn fire_timeout_with_none_duration_should_behave_like_fire() {
            let (mut channel, mut server, _post_office) = setup(100);

            let req = Request::new(0);
            channel.fire_timeout(req, None).await.unwrap();

            let _frame = server.recv().await.unwrap();
        }

        #[test(tokio::test)]
        async fn concurrent_requests_should_each_get_their_own_response() {
            let (mut channel1, _server, post_office) = setup(100);
            let mut channel2 = channel1.clone();

            let req1 = Request::new(10u8);
            let req2 = Request::new(20u8);
            let res1 = Response::new(req1.id.clone(), 11u8);
            let res2 = Response::new(req2.id.clone(), 22u8);

            // Send both requests concurrently and get their mailboxes
            let mut mb1 = channel1.mail(req1).await.unwrap();
            let mut mb2 = channel2.mail(req2).await.unwrap();

            // Deliver responses (out of order -- res2 first)
            post_office
                .deliver_untyped_response(res2.to_untyped_response().unwrap().into_owned())
                .await;
            post_office
                .deliver_untyped_response(res1.to_untyped_response().unwrap().into_owned())
                .await;

            // Each mailbox should get its own response, not the other's
            assert_eq!(mb1.next().await, Some(res1));
            assert_eq!(mb2.next().await, Some(res2));
        }

        #[test(tokio::test)]
        async fn typed_channel_should_skip_responses_that_fail_deserialization() {
            // Use Channel<u8, u8> but deliver a response whose payload is a string, not u8
            let post_office = Arc::new(PostOffice::default());
            let (tx, _rx) = mpsc::channel(100);
            let channel: Channel<u8, u8> = UntypedChannel {
                tx,
                post_office: Arc::downgrade(&post_office),
            }
            .into_typed_channel();

            // Create a default mailbox on the typed channel
            let mut mailbox = channel.assign_default_mailbox(10).await.unwrap();

            // Deliver an untyped response with an invalid payload (string "hello" instead of u8)
            let bad_payload =
                crate::net::common::utils::serialize_to_vec(&"hello".to_string()).unwrap();
            let bad_response = UntypedResponse {
                header: std::borrow::Cow::Owned(vec![]),
                id: std::borrow::Cow::Owned("resp1".to_string()),
                origin_id: std::borrow::Cow::Owned("".to_string()),
                payload: std::borrow::Cow::Owned(bad_payload),
            };

            // Then deliver a valid response
            let good_payload = crate::net::common::utils::serialize_to_vec(&42u8).unwrap();
            let good_response = UntypedResponse {
                header: std::borrow::Cow::Owned(vec![]),
                id: std::borrow::Cow::Owned("resp2".to_string()),
                origin_id: std::borrow::Cow::Owned("".to_string()),
                payload: std::borrow::Cow::Owned(good_payload),
            };

            post_office.deliver_untyped_response(bad_response).await;
            post_office.deliver_untyped_response(good_response).await;

            // The typed mailbox should skip the bad response and return the good one
            let resp = mailbox.next().await.unwrap();
            assert_eq!(resp.payload, 42u8);
        }

        #[test(tokio::test)]
        async fn clone_should_produce_independent_channel_sharing_same_transport() {
            let (mut channel, mut server, _post_office) = setup(100);
            let mut cloned = channel.clone();

            // Both channels can fire requests on the same transport
            let req1 = Request::new(1u8);
            let req2 = Request::new(2u8);
            channel.fire(req1).await.unwrap();
            cloned.fire(req2).await.unwrap();

            // Server should receive both
            let _frame1 = server.recv().await.unwrap();
            let _frame2 = server.recv().await.unwrap();
        }
    }

    mod untyped {
        use std::sync::Arc;
        use std::time::Duration;

        use test_log::test;

        use super::*;

        type TestChannel = UntypedChannel;
        type Setup = (
            TestChannel,
            mpsc::Receiver<UntypedRequest<'static>>,
            Arc<PostOffice<UntypedResponse<'static>>>,
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

            let req = Request::new(0).to_untyped_request().unwrap().into_owned();
            let res = Response::new(req.id.clone().into_owned(), 1)
                .to_untyped_response()
                .unwrap()
                .into_owned();

            let mut mailbox = channel.mail(req).await.unwrap();

            // Send and receive first response
            assert!(
                post_office.deliver_untyped_response(res.clone()).await,
                "Failed to deliver: {res:?}"
            );
            assert_eq!(mailbox.next().await, Some(res.clone()));

            // Send and receive second response
            assert!(
                post_office.deliver_untyped_response(res.clone()).await,
                "Failed to deliver: {res:?}"
            );
            assert_eq!(mailbox.next().await, Some(res.clone()));

            // Trigger the mailbox to wait BEFORE closing our mailbox to ensure that
            // we don't get stuck if the mailbox was already waiting
            let next_task = tokio::spawn(async move { mailbox.next().await });
            tokio::task::yield_now().await;

            // Close our specific mailbox
            post_office
                .cancel(&res.origin_id.clone().into_owned())
                .await;

            match next_task.await {
                Ok(None) => {}
                x => panic!("Unexpected response: {:?}", x),
            }
        }

        #[test(tokio::test)]
        async fn send_should_wait_until_response_received() {
            let (mut channel, _server, post_office) = setup(100);

            let req = Request::new(0).to_untyped_request().unwrap().into_owned();
            let res = Response::new(req.id.clone().into_owned(), 1)
                .to_untyped_response()
                .unwrap()
                .into_owned();

            let (actual, _) = tokio::join!(
                channel.send(req),
                post_office.deliver_untyped_response(res.clone())
            );
            match actual {
                Ok(actual) => assert_eq!(actual, res),
                x => panic!("Unexpected response: {:?}", x),
            }
        }

        #[test(tokio::test)]
        async fn send_timeout_should_fail_if_response_not_received_in_time() {
            let (mut channel, mut server, _post_office) = setup(100);

            let req = Request::new(0).to_untyped_request().unwrap().into_owned();
            match channel.send_timeout(req, Duration::from_millis(30)).await {
                Err(x) => assert_eq!(x.kind(), io::ErrorKind::TimedOut),
                x => panic!("Unexpected response: {:?}", x),
            }

            let _frame = server.recv().await.unwrap();
        }

        #[test(tokio::test)]
        async fn fire_should_send_request_and_not_wait_for_response() {
            let (mut channel, mut server, _post_office) = setup(100);

            let req = Request::new(0).to_untyped_request().unwrap().into_owned();
            match channel.fire(req).await {
                Ok(_) => {}
                x => panic!("Unexpected response: {:?}", x),
            }

            let _frame = server.recv().await.unwrap();
        }

        #[test(tokio::test)]
        async fn is_closed_should_return_false_when_receiver_exists() {
            let (channel, _server, _post_office) = setup(100);
            assert!(!channel.is_closed());
        }

        #[test(tokio::test)]
        async fn is_closed_should_return_true_when_receiver_dropped() {
            let (channel, server, _post_office) = setup(100);
            drop(server);
            assert!(channel.is_closed());
        }

        #[test(tokio::test)]
        async fn into_typed_channel_should_produce_working_channel() {
            let (channel, mut server, post_office) = setup(100);
            let mut typed: Channel<u8, u8> = channel.into_typed_channel();

            let req = Request::new(0u8);
            let res = Response::new(req.id.clone(), 1u8);

            let (actual, _) = tokio::join!(
                typed.send(req),
                post_office
                    .deliver_untyped_response(res.to_untyped_response().unwrap().into_owned())
            );
            assert_eq!(actual.unwrap(), res);

            let _frame = server.recv().await.unwrap();
        }

        #[test(tokio::test)]
        async fn fire_should_fail_with_broken_pipe_when_receiver_dropped() {
            let (mut channel, server, _post_office) = setup(100);
            drop(server);

            let req = Request::new(0).to_untyped_request().unwrap().into_owned();
            let err = channel.fire(req).await.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        }

        #[test(tokio::test)]
        async fn mail_should_fail_when_post_office_dropped() {
            let (mut channel, _server, post_office) = setup(100);
            drop(post_office);

            let req = Request::new(0).to_untyped_request().unwrap().into_owned();
            match channel.mail(req).await {
                Err(err) => assert_eq!(err.kind(), io::ErrorKind::NotConnected),
                Ok(_) => panic!("Expected NotConnected error"),
            }
        }

        #[test(tokio::test)]
        async fn send_should_fail_with_connection_aborted_when_mailbox_cancelled_before_response() {
            let (mut channel, _server, post_office) = setup(100);

            let req = Request::new(0u8).to_untyped_request().unwrap().into_owned();
            let req_id = req.id.clone().into_owned();

            let po = post_office.clone();
            let cancel_task = tokio::spawn(async move {
                tokio::task::yield_now().await;
                po.cancel(&req_id).await;
            });

            let result = channel.send(req).await;
            cancel_task.await.unwrap();

            let err = result.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted);
        }

        #[test(tokio::test)]
        async fn assign_default_mailbox_should_fail_when_post_office_dropped() {
            let (channel, _server, post_office) = setup(100);
            drop(post_office);

            match channel.assign_default_mailbox(10).await {
                Err(err) => assert_eq!(err.kind(), io::ErrorKind::NotConnected),
                Ok(_) => panic!("Expected NotConnected error"),
            }
        }

        #[test(tokio::test)]
        async fn remove_default_mailbox_should_fail_when_post_office_dropped() {
            let (channel, _server, post_office) = setup(100);
            drop(post_office);

            let err = channel.remove_default_mailbox().await.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotConnected);
        }

        #[test(tokio::test)]
        async fn mail_timeout_should_fail_if_fire_blocks_past_deadline() {
            // Buffer of 1 means the second fire will block
            let (mut channel, _server, _post_office) = setup(1);

            // Fill the channel buffer so the next fire will block
            let req_fill = Request::new(0u8).to_untyped_request().unwrap().into_owned();
            channel.fire(req_fill).await.unwrap();

            // Now mail_timeout should time out because fire cannot complete
            let req = Request::new(1u8).to_untyped_request().unwrap().into_owned();
            match channel.mail_timeout(req, Duration::from_millis(30)).await {
                Err(err) => assert_eq!(err.kind(), io::ErrorKind::TimedOut),
                Ok(_) => panic!("Expected TimedOut error"),
            }
        }

        #[test(tokio::test)]
        async fn mail_timeout_with_none_duration_should_behave_like_mail() {
            let (mut channel, _server, post_office) = setup(100);

            let req = Request::new(0u8).to_untyped_request().unwrap().into_owned();
            let res = Response::new(req.id.clone().into_owned(), 1u8)
                .to_untyped_response()
                .unwrap()
                .into_owned();

            let (mailbox_result, _) = tokio::join!(
                channel.mail_timeout(req, None),
                post_office.deliver_untyped_response(res.clone())
            );

            match mailbox_result {
                Ok(mut mailbox) => assert_eq!(mailbox.next().await, Some(res)),
                Err(err) => panic!("Expected Ok, got Err: {err}"),
            }
        }

        #[test(tokio::test)]
        async fn send_timeout_with_none_duration_should_behave_like_send() {
            let (mut channel, _server, post_office) = setup(100);

            let req = Request::new(0u8).to_untyped_request().unwrap().into_owned();
            let res = Response::new(req.id.clone().into_owned(), 1u8)
                .to_untyped_response()
                .unwrap()
                .into_owned();

            let (actual, _) = tokio::join!(
                channel.send_timeout(req, None),
                post_office.deliver_untyped_response(res.clone())
            );
            assert_eq!(actual.unwrap(), res);
        }

        #[test(tokio::test)]
        async fn fire_timeout_should_succeed_when_within_time() {
            let (mut channel, mut server, _post_office) = setup(100);

            let req = Request::new(0).to_untyped_request().unwrap().into_owned();
            channel
                .fire_timeout(req, Duration::from_secs(1))
                .await
                .unwrap();

            let _frame = server.recv().await.unwrap();
        }

        #[test(tokio::test)]
        async fn fire_timeout_with_none_duration_should_behave_like_fire() {
            let (mut channel, mut server, _post_office) = setup(100);

            let req = Request::new(0).to_untyped_request().unwrap().into_owned();
            channel.fire_timeout(req, None).await.unwrap();

            let _frame = server.recv().await.unwrap();
        }

        #[test(tokio::test)]
        async fn concurrent_requests_should_each_get_their_own_response() {
            let (mut channel1, _server, post_office) = setup(100);
            let mut channel2 = channel1.clone();

            let req1 = Request::new(10u8)
                .to_untyped_request()
                .unwrap()
                .into_owned();
            let req2 = Request::new(20u8)
                .to_untyped_request()
                .unwrap()
                .into_owned();
            let res1 = Response::new(req1.id.clone().into_owned(), 11u8)
                .to_untyped_response()
                .unwrap()
                .into_owned();
            let res2 = Response::new(req2.id.clone().into_owned(), 22u8)
                .to_untyped_response()
                .unwrap()
                .into_owned();

            let mut mb1 = channel1.mail(req1).await.unwrap();
            let mut mb2 = channel2.mail(req2).await.unwrap();

            // Deliver responses out of order
            post_office.deliver_untyped_response(res2.clone()).await;
            post_office.deliver_untyped_response(res1.clone()).await;

            assert_eq!(mb1.next().await, Some(res1));
            assert_eq!(mb2.next().await, Some(res2));
        }

        #[test(tokio::test)]
        async fn clone_should_produce_independent_channel_sharing_same_transport() {
            let (mut channel, mut server, _post_office) = setup(100);
            let mut cloned = channel.clone();

            let req1 = Request::new(1u8).to_untyped_request().unwrap().into_owned();
            let req2 = Request::new(2u8).to_untyped_request().unwrap().into_owned();
            channel.fire(req1).await.unwrap();
            cloned.fire(req2).await.unwrap();

            let _frame1 = server.recv().await.unwrap();
            let _frame2 = server.recv().await.unwrap();
        }

        #[test(tokio::test)]
        async fn mail_should_fail_with_broken_pipe_when_fire_fails_after_mailbox_created() {
            // Create channel with buffer=1 so we can fill it
            let (mut channel, server, _post_office) = setup(1);

            // Fill the channel buffer
            let req_fill = Request::new(0u8).to_untyped_request().unwrap().into_owned();
            channel.fire(req_fill).await.unwrap();

            // Drop the server so the channel is closed
            drop(server);

            // Now mail should fail because fire fails (receiver is dropped)
            let req = Request::new(1u8).to_untyped_request().unwrap().into_owned();
            match channel.mail(req).await {
                Err(err) => assert_eq!(err.kind(), io::ErrorKind::BrokenPipe),
                Ok(_) => panic!("Expected BrokenPipe error"),
            }
        }
    }
}
