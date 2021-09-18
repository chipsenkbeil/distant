use crate::{
    client::utils,
    constants::CLIENT_MAILBOX_CAPACITY,
    data::{Request, Response},
    net::{Codec, DataStream, Transport, TransportError},
};
use log::*;
use std::{
    convert,
    net::SocketAddr,
    ops::{Deref, DerefMut},
    sync::Arc,
};
use tokio::{
    io,
    net::TcpStream,
    sync::{mpsc, Mutex},
    task::{JoinError, JoinHandle},
    time::Duration,
};

mod ext;
pub use ext::SessionExt;

mod info;
pub use info::{SessionInfo, SessionInfoFile, SessionInfoParseError};

mod mailbox;
pub use mailbox::Mailbox;
use mailbox::PostOffice;

/// Represents a session with a remote server that can be used to send requests & receive responses
pub struct Session {
    /// Used to send requests to a server
    sender: SessionSender,

    /// Contains the task that is running to send requests to a server
    request_task: JoinHandle<()>,

    /// Contains the task that is running to receive responses from a server
    response_task: JoinHandle<()>,

    /// Contains the task that runs on a timer to prune closed mailboxes
    prune_task: JoinHandle<()>,
}

impl Session {
    /// Connect to a remote TCP server using the provided information
    pub async fn tcp_connect<U>(addr: SocketAddr, codec: U) -> io::Result<Self>
    where
        U: Codec + Send + 'static,
    {
        let transport = Transport::<TcpStream, U>::connect(addr, codec).await?;
        debug!(
            "Session has been established with {}",
            transport
                .peer_addr()
                .map(|x| x.to_string())
                .unwrap_or_else(|_| String::from("???"))
        );
        Self::initialize(transport)
    }

    /// Connect to a remote TCP server, timing out after duration has passed
    pub async fn tcp_connect_timeout<U>(
        addr: SocketAddr,
        codec: U,
        duration: Duration,
    ) -> io::Result<Self>
    where
        U: Codec + Send + 'static,
    {
        utils::timeout(duration, Self::tcp_connect(addr, codec))
            .await
            .and_then(convert::identity)
    }
}

#[cfg(unix)]
impl Session {
    /// Connect to a proxy unix socket
    pub async fn unix_connect<U>(path: impl AsRef<std::path::Path>, codec: U) -> io::Result<Self>
    where
        U: Codec + Send + 'static,
    {
        let transport = Transport::<tokio::net::UnixStream, U>::connect(path, codec).await?;
        debug!(
            "Session has been established with {}",
            transport
                .peer_addr()
                .map(|x| format!("{:?}", x))
                .unwrap_or_else(|_| String::from("???"))
        );
        Self::initialize(transport)
    }

    /// Connect to a proxy unix socket, timing out after duration has passed
    pub async fn unix_connect_timeout<U>(
        path: impl AsRef<std::path::Path>,
        codec: U,
        duration: Duration,
    ) -> io::Result<Self>
    where
        U: Codec + Send + 'static,
    {
        utils::timeout(duration, Self::unix_connect(path, codec))
            .await
            .and_then(convert::identity)
    }
}

impl Session {
    /// Initializes a session using the provided transport
    pub fn initialize<T, U>(transport: Transport<T, U>) -> io::Result<Self>
    where
        T: DataStream,
        U: Codec + Send + 'static,
    {
        let (mut t_read, mut t_write) = transport.into_split();
        let post_office = Arc::new(Mutex::new(PostOffice::new()));

        // Start a task that continually checks for responses and triggers callbacks
        let post_office_2 = Arc::clone(&post_office);
        let response_task = tokio::spawn(async move {
            loop {
                match t_read.receive::<Response>().await {
                    Ok(Some(res)) => {
                        trace!("Incoming response: {:?}", res);
                        let res_id = res.id;
                        let res_origin_id = res.origin_id;

                        // Try to send response to appropriate mailbox
                        // NOTE: We don't log failures as errors as using fire(...) for a
                        //       session is valid and would not have a mailbox
                        if !post_office_2.lock().await.deliver(res).await {
                            trace!(
                                "Response {} has no mailbox for origin {}",
                                res_id,
                                res_origin_id
                            );
                        }
                    }
                    Ok(None) => {
                        debug!("Session closing response task as transport read-half closed!");
                        break;
                    }
                    Err(x) => {
                        error!("{}", x);
                        break;
                    }
                }
            }

            // Clean up remaining mailbox senders
            post_office_2.lock().await.close_mailboxes();
        });

        let (tx, mut rx) = mpsc::channel::<Request>(1);
        let request_task = tokio::spawn(async move {
            while let Some(req) = rx.recv().await {
                if let Err(x) = t_write.send(req).await {
                    error!("Failed to send request to server: {}", x);
                    break;
                }
            }
        });

        // Create a task that runs once a minute and prunes mailboxes
        let post_office_2 = Arc::clone(&post_office);
        let prune_task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                post_office_2.lock().await.prune_mailboxes();
            }
        });

        let sender = SessionSender { tx, post_office };

        Ok(Self {
            sender,
            request_task,
            response_task,
            prune_task,
        })
    }
}

impl Session {
    /// Waits for the session to terminate, which results when the receiving end of the network
    /// connection is closed (or the session is shutdown)
    pub async fn wait(self) -> Result<(), JoinError> {
        self.prune_task.abort();
        tokio::try_join!(self.request_task, self.response_task).map(|_| ())
    }

    /// Abort the session's current connection by forcing its response task to shutdown
    pub fn abort(&self) {
        self.request_task.abort();
        self.response_task.abort();
        self.prune_task.abort();
    }

    /// Clones the underlying sender for requests and returns the cloned instance
    pub fn clone_sender(&self) -> SessionSender {
        self.sender.clone()
    }
}

impl Deref for Session {
    type Target = SessionSender;

    fn deref(&self) -> &Self::Target {
        &self.sender
    }
}

impl DerefMut for Session {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.sender
    }
}

/// Represents a sender of requests tied to a session
#[derive(Clone)]
pub struct SessionSender {
    /// Used to send requests to a server
    tx: mpsc::Sender<Request>,

    /// Collection of mailboxes for receiving responses to requests
    post_office: Arc<Mutex<PostOffice>>,
}

impl SessionSender {
    /// Returns true if no more requests can be transferred
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Sends a request and returns a mailbox that can receive one or more responses
    pub async fn mail(&mut self, req: Request) -> Result<Mailbox, TransportError> {
        trace!("Mailing request: {:?}", req);

        // First, create a mailbox using the request's id
        let mailbox = self
            .post_office
            .lock()
            .await
            .make_mailbox(req.id, CLIENT_MAILBOX_CAPACITY);

        // Second, send the request
        self.fire(req).await?;

        // Third, return mailbox
        Ok(mailbox)
    }

    /// Sends a request and waits for a response
    pub async fn send(&mut self, req: Request) -> Result<Response, TransportError> {
        trace!("Sending request: {:?}", req);

        // Send mail and get back a mailbox
        let mailbox = self.mail(req).await?;

        // Wait for first response, and then drop the mailbox
        mailbox.next().await.ok_or_else(|| {
            TransportError::IoError(io::Error::from(io::ErrorKind::ConnectionAborted))
        })
    }

    /// Sends a request and waits for a response, timing out after duration has passed
    pub async fn send_timeout(
        &mut self,
        req: Request,
        duration: Duration,
    ) -> Result<Response, TransportError> {
        utils::timeout(duration, self.send(req))
            .await
            .map_err(TransportError::from)
            .and_then(convert::identity)
    }

    /// Sends a request without waiting for a response
    ///
    /// Any response that would be received gets sent over the broadcast channel instead
    pub async fn fire(&mut self, req: Request) -> Result<(), TransportError> {
        trace!("Firing off request: {:?}", req);
        self.tx
            .send(req)
            .await
            .map_err(|x| TransportError::IoError(io::Error::new(io::ErrorKind::BrokenPipe, x)))
    }

    /// Sends a request without waiting for a response, timing out after duration has passed
    pub async fn fire_timeout(
        &mut self,
        req: Request,
        duration: Duration,
    ) -> Result<(), TransportError> {
        utils::timeout(duration, self.fire(req))
            .await
            .map_err(TransportError::from)
            .and_then(convert::identity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        constants::test::TENANT,
        data::{RequestData, ResponseData},
    };
    use std::time::Duration;

    #[tokio::test]
    async fn send_should_wait_until_response_received() {
        let (t1, mut t2) = Transport::make_pair();
        let mut session = Session::initialize(t1).unwrap();

        let req = Request::new(TENANT, vec![RequestData::ProcList {}]);
        let res = Response::new(
            TENANT,
            req.id,
            vec![ResponseData::ProcEntries {
                entries: Vec::new(),
            }],
        );

        let (actual, _) = tokio::join!(session.send(req), t2.send(res.clone()));
        match actual {
            Ok(actual) => assert_eq!(actual, res),
            x => panic!("Unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn send_timeout_should_fail_if_response_not_received_in_time() {
        let (t1, mut t2) = Transport::make_pair();
        let mut session = Session::initialize(t1).unwrap();

        let req = Request::new(TENANT, vec![RequestData::ProcList {}]);
        match session.send_timeout(req, Duration::from_millis(30)).await {
            Err(TransportError::IoError(x)) => assert_eq!(x.kind(), io::ErrorKind::TimedOut),
            x => panic!("Unexpected response: {:?}", x),
        }

        let req = t2.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(req.tenant, TENANT);
    }

    #[tokio::test]
    async fn fire_should_send_request_and_not_wait_for_response() {
        let (t1, mut t2) = Transport::make_pair();
        let mut session = Session::initialize(t1).unwrap();

        let req = Request::new(TENANT, vec![RequestData::ProcList {}]);
        match session.fire(req).await {
            Ok(_) => {}
            x => panic!("Unexpected response: {:?}", x),
        }

        let req = t2.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(req.tenant, TENANT);
    }
}
