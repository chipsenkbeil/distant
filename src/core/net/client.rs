use crate::core::{
    constants::CLIENT_BROADCAST_CHANNEL_CAPACITY,
    data::{Request, Response},
    net::{DataStream, Transport, TransportError, TransportWriteHalf},
    session::Session,
    utils,
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
    sync::{broadcast, oneshot},
    task::{JoinError, JoinHandle},
    time::Duration,
};
use tokio_stream::wrappers::BroadcastStream;

type Callbacks = Arc<Mutex<HashMap<usize, oneshot::Sender<Response>>>>;

/// Represents a client that can make requests against a server
pub struct Client<T>
where
    T: DataStream,
{
    /// Underlying transport used by client
    t_write: TransportWriteHalf<T::Write>,

    /// Collection of callbacks to be invoked upon receiving a response to a request
    callbacks: Callbacks,

    /// Callback to trigger when a response is received without an origin or with an origin
    /// not found in the list of callbacks
    broadcast: broadcast::Sender<Response>,

    /// Represents an initial receiver for broadcasted responses that can capture responses
    /// prior to a stream being established and consumed
    init_broadcast_receiver: Option<broadcast::Receiver<Response>>,

    /// Contains the task that is running to receive responses from a server
    response_task: JoinHandle<()>,
}

impl Client<TcpStream> {
    /// Connect to a remote TCP session
    pub async fn tcp_connect(session: Session) -> io::Result<Self> {
        let transport = Transport::<TcpStream>::connect(session).await?;
        debug!(
            "Client has connected to {}",
            transport
                .peer_addr()
                .map(|x| x.to_string())
                .unwrap_or_else(|_| String::from("???"))
        );
        Self::inner_connect(transport).await
    }

    /// Connect to a remote TCP session, timing out after duration has passed
    pub async fn tcp_connect_timeout(session: Session, duration: Duration) -> io::Result<Self> {
        utils::timeout(duration, Self::tcp_connect(session))
            .await
            .and_then(convert::identity)
    }
}

#[cfg(unix)]
impl Client<tokio::net::UnixStream> {
    /// Connect to a proxy unix socket
    pub async fn unix_connect(
        path: impl AsRef<std::path::Path>,
        auth_key: Option<Arc<orion::aead::SecretKey>>,
    ) -> io::Result<Self> {
        let transport = Transport::<tokio::net::UnixStream>::connect(path, auth_key).await?;
        debug!(
            "Client has connected to {}",
            transport
                .peer_addr()
                .map(|x| format!("{:?}", x))
                .unwrap_or_else(|_| String::from("???"))
        );
        Self::inner_connect(transport).await
    }

    /// Connect to a proxy unix socket, timing out after duration has passed
    pub async fn unix_connect_timeout(
        path: impl AsRef<std::path::Path>,
        auth_key: Option<Arc<orion::aead::SecretKey>>,
        duration: Duration,
    ) -> io::Result<Self> {
        utils::timeout(duration, Self::unix_connect(path, auth_key))
            .await
            .and_then(convert::identity)
    }
}

impl<T> Client<T>
where
    T: DataStream,
{
    /// Establishes a connection using the provided session
    async fn inner_connect(transport: Transport<T>) -> io::Result<Self> {
        let (mut t_read, t_write) = transport.into_split();
        let callbacks: Callbacks = Arc::new(Mutex::new(HashMap::new()));
        let (broadcast, init_broadcast_receiver) =
            broadcast::channel(CLIENT_BROADCAST_CHANNEL_CAPACITY);

        // Start a task that continually checks for responses and triggers callbacks
        let callbacks_2 = Arc::clone(&callbacks);
        let broadcast_2 = broadcast.clone();
        let response_task = tokio::spawn(async move {
            loop {
                match t_read.receive::<Response>().await {
                    Ok(Some(res)) => {
                        trace!("Client got response: {:?}", res);
                        let maybe_callback = res
                            .origin_id
                            .as_ref()
                            .and_then(|id| callbacks_2.lock().unwrap().remove(id));

                        // If there is an origin to this response, trigger the callback
                        if let Some(tx) = maybe_callback {
                            trace!("Client has callback! Triggering!");
                            if let Err(res) = tx.send(res) {
                                error!("Failed to trigger callback for response {}", res.id);
                            }

                        // Otherwise, this goes into the junk draw of response handlers
                        } else {
                            trace!("Client does not have callback! Broadcasting!");
                            if let Err(x) = broadcast_2.send(res) {
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
            broadcast,
            init_broadcast_receiver: Some(init_broadcast_receiver),
            response_task,
        })
    }

    /// Waits for the client to terminate, which results when the receiving end of the network
    /// connection is closed (or the client is shutdown)
    pub async fn wait(self) -> Result<(), JoinError> {
        self.response_task.await
    }

    /// Abort the client's current connection by forcing its response task to shutdown
    pub fn abort(&self) {
        self.response_task.abort()
    }

    /// Sends a request and waits for a response
    pub async fn send(&mut self, req: Request) -> Result<Response, TransportError> {
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

    /// Clones a new instance of the broadcaster used by the client
    pub fn to_response_broadcaster(&self) -> broadcast::Sender<Response> {
        self.broadcast.clone()
    }

    /// Creates and returns a new stream of responses that are received that do not match the
    /// response to a `send` request
    pub fn to_response_broadcast_stream(&mut self) -> BroadcastStream<Response> {
        BroadcastStream::new(
            self.init_broadcast_receiver
                .take()
                .unwrap_or_else(|| self.broadcast.subscribe()),
        )
    }
}
