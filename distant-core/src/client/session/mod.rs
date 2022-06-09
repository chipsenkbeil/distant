use crate::{
    client::utils,
    constants::CLIENT_MAILBOX_CAPACITY,
    data::{Request, Response},
    net::{Codec, DataStream, Transport, TransportError},
};
use log::*;
use serde::{Deserialize, Serialize};
use std::{
    convert,
    net::SocketAddr,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::{Arc, Weak},
};
use tokio::{
    io,
    net::TcpStream,
    sync::{mpsc, Mutex},
    task::{JoinError, JoinHandle},
    time::Duration,
};

mod ext;
pub use ext::{SessionChannelExt, SessionChannelExtError};

mod info;
pub use info::{SessionInfo, SessionInfoFile, SessionInfoParseError};

mod mailbox;
pub use mailbox::Mailbox;
use mailbox::PostOffice;

/// Details about the session
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionDetails {
    /// Indicates session is a TCP type
    Tcp { addr: SocketAddr },

    /// Indicates session is a Unix socket type
    Socket { path: PathBuf },

    /// Indicates session type is inmemory
    Inmemory,

    /// Indicates session type is a custom type (such as ssh)
    Custom { tag: String },
}

impl SessionDetails {
    /// Represents the tag associated with the session
    pub fn tag(&self) -> Option<&str> {
        match self {
            Self::Custom { tag } => Some(tag.as_str()),
            _ => None,
        }
    }

    /// Represents the socket address associated with the session, if it has one
    pub fn addr(&self) -> Option<SocketAddr> {
        match self {
            Self::Tcp { addr, .. } => Some(*addr),
            _ => None,
        }
    }

    /// Represents the path associated with the session, if it has one
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Socket { path, .. } => Some(path.as_path()),
            _ => None,
        }
    }
}

/// Represents a session with a remote server that can be used to send requests & receive responses
pub struct Session {
    /// Used to send requests to a server
    channel: SessionChannel,

    /// Details about the session
    details: Option<SessionDetails>,

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
        let details = SessionDetails::Tcp { addr };
        debug!("Session has been established with {}", addr);
        Self::initialize_with_details(transport, Some(details))
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

    /// Convert into underlying channel
    pub fn into_channel(self) -> SessionChannel {
        self.channel
    }
}

#[cfg(unix)]
impl Session {
    /// Connect to a proxy unix socket
    pub async fn unix_connect<U>(path: impl AsRef<std::path::Path>, codec: U) -> io::Result<Self>
    where
        U: Codec + Send + 'static,
    {
        let p = path.as_ref();
        let transport = Transport::<tokio::net::UnixStream, U>::connect(p, codec).await?;
        let details = SessionDetails::Socket {
            path: p.to_path_buf(),
        };
        debug!("Session has been established with {:?}", p);
        Self::initialize_with_details(transport, Some(details))
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
    /// Initializes a session using the provided transport and no extra details
    pub fn initialize<T, U>(transport: Transport<T, U>) -> io::Result<Self>
    where
        T: DataStream,
        U: Codec + Send + 'static,
    {
        Self::initialize_with_details(transport, None)
    }

    /// Initializes a session using the provided transport and extra details
    pub fn initialize_with_details<T, U>(
        transport: Transport<T, U>,
        details: Option<SessionDetails>,
    ) -> io::Result<Self>
    where
        T: DataStream,
        U: Codec + Send + 'static,
    {
        let (mut t_read, mut t_write) = transport.into_split();
        let post_office = Arc::new(Mutex::new(PostOffice::new()));
        let weak_post_office = Arc::downgrade(&post_office);

        // Start a task that continually checks for responses and delivers them using the
        // post office
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
                        if !post_office.lock().await.deliver(res).await {
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
                        error!("Failed to receive response from server: {}", x);
                        break;
                    }
                }
            }

            // Clean up remaining mailbox senders
            post_office.lock().await.clear_mailboxes();
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
        let post_office = Weak::clone(&weak_post_office);
        let prune_task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                if let Some(post_office) = Weak::upgrade(&post_office) {
                    post_office.lock().await.prune_mailboxes();
                } else {
                    break;
                }
            }
        });

        let channel = SessionChannel {
            tx,
            post_office: weak_post_office,
        };

        Ok(Self {
            channel,
            details,
            request_task,
            response_task,
            prune_task,
        })
    }
}

impl Session {
    /// Returns details about the session, if it has any
    pub fn details(&self) -> Option<&SessionDetails> {
        self.details.as_ref()
    }

    /// Waits for the session to terminate, which results when the receiving end of the network
    /// connection is closed (or the session is shutdown)
    pub async fn wait(self) -> Result<(), JoinError> {
        self.prune_task.abort();
        tokio::try_join!(self.request_task, self.response_task).map(|_| ())
    }

    /// Abort the session's current connection by forcing its tasks to abort
    pub fn abort(&self) {
        self.request_task.abort();
        self.response_task.abort();
        self.prune_task.abort();
    }

    /// Clones the underlying channel for requests and returns the cloned instance
    pub fn clone_channel(&self) -> SessionChannel {
        self.channel.clone()
    }
}

impl Deref for Session {
    type Target = SessionChannel;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

impl DerefMut for Session {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.channel
    }
}

impl From<Session> for SessionChannel {
    fn from(session: Session) -> Self {
        session.channel
    }
}

/// Represents a sender of requests tied to a session, holding onto a weak reference of
/// mailboxes to relay responses, meaning that once the [`Session`] is closed or dropped,
/// any sent request will no longer be able to receive responses
#[derive(Clone)]
pub struct SessionChannel {
    /// Used to send requests to a server
    tx: mpsc::Sender<Request>,

    /// Collection of mailboxes for receiving responses to requests
    post_office: Weak<Mutex<PostOffice>>,
}

impl SessionChannel {
    /// Returns true if no more requests can be transferred
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    /// Sends a request and returns a mailbox that can receive one or more responses, failing if
    /// unable to send a request or if the session's receiving line to the remote server has
    /// already been severed
    pub async fn mail(&mut self, req: Request) -> Result<Mailbox, TransportError> {
        trace!("Mailing request: {:?}", req);

        // First, create a mailbox using the request's id
        let mailbox = Weak::upgrade(&self.post_office)
            .ok_or_else(|| {
                TransportError::IoError(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "Session's post office is no longer available",
                ))
            })?
            .lock()
            .await
            .make_mailbox(req.id, CLIENT_MAILBOX_CAPACITY);

        // Second, send the request
        self.fire(req).await?;

        // Third, return mailbox
        Ok(mailbox)
    }

    /// Sends a request and waits for a response, failing if unable to send a request or if
    /// the session's receiving line to the remote server has already been severed
    pub async fn send(&mut self, req: Request) -> Result<Response, TransportError> {
        trace!("Sending request: {:?}", req);

        // Send mail and get back a mailbox
        let mut mailbox = self.mail(req).await?;

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

    /// Sends a request without waiting for a response; this method is able to be used even
    /// if the session's receiving line to the remote server has been severed
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
    async fn mail_should_return_mailbox_that_receives_responses_until_transport_closes() {
        let (t1, mut t2) = Transport::make_pair();
        let mut session = Session::initialize(t1).unwrap();

        let req = Request::new(TENANT, vec![RequestData::ProcList {}]);
        let res = Response::new(TENANT, req.id, vec![ResponseData::Ok]);

        let mut mailbox = session.mail(req).await.unwrap();

        // Get first response
        match tokio::join!(mailbox.next(), t2.send(res.clone())) {
            (Some(actual), _) => assert_eq!(actual, res),
            x => panic!("Unexpected response: {:?}", x),
        }

        // Get second response
        match tokio::join!(mailbox.next(), t2.send(res.clone())) {
            (Some(actual), _) => assert_eq!(actual, res),
            x => panic!("Unexpected response: {:?}", x),
        }

        // Trigger the mailbox to wait BEFORE closing our transport to ensure that
        // we don't get stuck if the mailbox was already waiting
        let next_task = tokio::spawn(async move { mailbox.next().await });
        tokio::task::yield_now().await;

        drop(t2);
        match next_task.await {
            Ok(None) => {}
            x => panic!("Unexpected response: {:?}", x),
        }
    }

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
