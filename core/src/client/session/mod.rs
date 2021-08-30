use crate::{
    client::utils,
    constants::CLIENT_BROADCAST_CHANNEL_CAPACITY,
    data::{Request, Response},
    net::{DataStream, InmemoryStream, SecretKey, Transport, TransportError, TransportWriteHalf},
};
use log::*;
use std::{
    collections::HashMap,
    convert,
    sync::{Arc, Mutex},
};
use tokio::{
    io,
    net::TcpStream,
    sync::{mpsc, oneshot},
    task::{JoinError, JoinHandle},
    time::Duration,
};

mod info;
pub use info::{SessionInfo, SessionInfoFile, SessionInfoParseError};

type Callbacks = Arc<Mutex<HashMap<usize, oneshot::Sender<Response>>>>;

/// Represents a session with a remote server that can be used to send requests & receive responses
pub struct Session<T>
where
    T: DataStream,
{
    /// Underlying transport used by session
    t_write: TransportWriteHalf<T::Write>,

    /// Collection of callbacks to be invoked upon receiving a response to a request
    callbacks: Callbacks,

    /// Contains the task that is running to receive responses from a server
    response_task: JoinHandle<()>,

    /// Represents the receiver for broadcasted responses (ones with no callback)
    pub broadcast: Option<mpsc::Receiver<Response>>,
}

impl Session<InmemoryStream> {
    /// Creates a session around an inmemory transport
    pub async fn from_inmemory_transport(transport: Transport<InmemoryStream>) -> io::Result<Self> {
        Self::initialize(transport).await
    }
}

impl Session<TcpStream> {
    /// Connect to a remote TCP server using the provided information
    pub async fn tcp_connect(info: SessionInfo) -> io::Result<Self> {
        let addr = info.to_socket_addr().await?;
        let transport =
            Transport::<TcpStream>::connect(addr, Some(Arc::new(info.auth_key))).await?;
        debug!(
            "Session has been established with {}",
            transport
                .peer_addr()
                .map(|x| x.to_string())
                .unwrap_or_else(|_| String::from("???"))
        );
        Self::initialize(transport).await
    }

    /// Connect to a remote TCP server, timing out after duration has passed
    pub async fn tcp_connect_timeout(info: SessionInfo, duration: Duration) -> io::Result<Self> {
        utils::timeout(duration, Self::tcp_connect(info))
            .await
            .and_then(convert::identity)
    }
}

#[cfg(unix)]
impl Session<tokio::net::UnixStream> {
    /// Connect to a proxy unix socket
    pub async fn unix_connect(
        path: impl AsRef<std::path::Path>,
        auth_key: Option<Arc<SecretKey>>,
    ) -> io::Result<Self> {
        let transport = Transport::<tokio::net::UnixStream>::connect(path, auth_key).await?;
        debug!(
            "Session has been established with {}",
            transport
                .peer_addr()
                .map(|x| format!("{:?}", x))
                .unwrap_or_else(|_| String::from("???"))
        );
        Self::initialize(transport).await
    }

    /// Connect to a proxy unix socket, timing out after duration has passed
    pub async fn unix_connect_timeout(
        path: impl AsRef<std::path::Path>,
        auth_key: Option<Arc<SecretKey>>,
        duration: Duration,
    ) -> io::Result<Self> {
        utils::timeout(duration, Self::unix_connect(path, auth_key))
            .await
            .and_then(convert::identity)
    }
}

impl<T> Session<T>
where
    T: DataStream,
{
    /// Initializes a session using the provided transport
    pub async fn initialize(transport: Transport<T>) -> io::Result<Self> {
        let (mut t_read, t_write) = transport.into_split();
        let callbacks: Callbacks = Arc::new(Mutex::new(HashMap::new()));
        let (broadcast_tx, broadcast_rx) = mpsc::channel(CLIENT_BROADCAST_CHANNEL_CAPACITY);

        // Start a task that continually checks for responses and triggers callbacks
        let callbacks_2 = Arc::clone(&callbacks);
        let response_task = tokio::spawn(async move {
            loop {
                match t_read.receive::<Response>().await {
                    Ok(Some(res)) => {
                        trace!("Incoming response: {:?}", res);
                        let maybe_callback = res
                            .origin_id
                            .as_ref()
                            .and_then(|id| callbacks_2.lock().unwrap().remove(id));

                        // If there is an origin to this response, trigger the callback
                        if let Some(tx) = maybe_callback {
                            trace!("Callback exists for response! Triggering!");
                            if let Err(res) = tx.send(res) {
                                error!("Failed to trigger callback for response {}", res.id);
                            }

                        // Otherwise, this goes into the junk draw of response handlers
                        } else {
                            trace!("Callback missing for response! Broadcasting!");
                            if let Err(x) = broadcast_tx.send(res).await {
                                error!("Failed to trigger broadcast: {}", x);
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(x) => {
                        error!("{}", x);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            t_write,
            callbacks,
            broadcast: Some(broadcast_rx),
            response_task,
        })
    }

    /// Waits for the session to terminate, which results when the receiving end of the network
    /// connection is closed (or the session is shutdown)
    pub async fn wait(self) -> Result<(), JoinError> {
        self.response_task.await
    }

    /// Abort the session's current connection by forcing its response task to shutdown
    pub fn abort(&self) {
        self.response_task.abort()
    }

    /// Sends a request and waits for a response
    pub async fn send(&mut self, req: Request) -> Result<Response, TransportError> {
        trace!("Sending request: {:?}", req);

        // First, add a callback that will trigger when we get the response for this request
        let (tx, rx) = oneshot::channel();
        self.callbacks.lock().unwrap().insert(req.id, tx);

        // Second, send the request
        self.t_write.send(req).await?;

        // Third, wait for the response
        rx.await
            .map_err(|x| TransportError::from(io::Error::new(io::ErrorKind::ConnectionAborted, x)))
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
        self.t_write.send(req).await
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
        let mut session = Session::initialize(t1).await.unwrap();

        let req = Request::new(TENANT, vec![RequestData::ProcList {}]);
        let res = Response::new(
            TENANT,
            Some(req.id),
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
        let mut session = Session::initialize(t1).await.unwrap();

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
        let mut session = Session::initialize(t1).await.unwrap();

        let req = Request::new(TENANT, vec![RequestData::ProcList {}]);
        match session.fire(req).await {
            Ok(_) => {}
            x => panic!("Unexpected response: {:?}", x),
        }

        let req = t2.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(req.tenant, TENANT);
    }
}
