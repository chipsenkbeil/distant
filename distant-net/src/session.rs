use crate::{Codec, DataStream, PostOffice, Request, Response, TcpStream, Transport};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    convert,
    marker::PhantomData,
    net::SocketAddr,
    ops::{Deref, DerefMut},
    sync::Arc,
};
use tokio::{
    io,
    sync::mpsc,
    task::{JoinError, JoinHandle},
    time::Duration,
};

mod channel;
pub use channel::*;

/// Represents a session with a remote server that can be used to send requests & receive responses
pub struct Session<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Used to send requests to a server
    channel: Channel<T, U>,

    /// Contains the task that is running to send requests to a server
    request_task: JoinHandle<()>,

    /// Contains the task that is running to receive responses from a server
    response_task: JoinHandle<()>,

    /// Type associated with responses
    _response_type: PhantomData<U>,
}

impl<T, U> Session<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Connect to a remote TCP server using the provided information
    pub async fn tcp_connect<C>(addr: SocketAddr, codec: C) -> io::Result<Self>
    where
        C: Codec + Send + 'static,
    {
        let stream = TcpStream::connect(addr).await?;
        let transport = Transport::new(stream, codec);
        Self::new(transport)
    }

    /// Connect to a remote TCP server, timing out after duration has passed
    pub async fn tcp_connect_timeout<C>(
        addr: SocketAddr,
        codec: C,
        duration: Duration,
    ) -> io::Result<Self>
    where
        C: Codec + Send + 'static,
    {
        tokio::time::timeout(duration, Self::tcp_connect(addr, codec))
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
            .and_then(convert::identity)
    }

    /// Convert into underlying channel
    pub fn into_channel(self) -> Channel<T, U> {
        self.channel
    }
}

#[cfg(unix)]
impl<T, U> Session<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Connect to a proxy unix socket
    pub async fn unix_connect<C>(path: impl AsRef<std::path::Path>, codec: C) -> io::Result<Self>
    where
        C: Codec + Send + 'static,
    {
        let p = path.as_ref();
        let stream = crate::UnixSocketStream::connect(p).await?;
        let transport = Transport::new(stream, codec);
        Self::new(transport)
    }

    /// Connect to a proxy unix socket, timing out after duration has passed
    pub async fn unix_connect_timeout<C>(
        path: impl AsRef<std::path::Path>,
        codec: C,
        duration: Duration,
    ) -> io::Result<Self>
    where
        C: Codec + Send + 'static,
    {
        tokio::time::timeout(duration, Self::unix_connect(path, codec))
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
            .and_then(convert::identity)
    }
}

impl<T, U> Session<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Initializes a session using the provided transport
    pub fn new<D, C>(transport: Transport<D, C>) -> io::Result<Self>
    where
        D: DataStream,
        C: Codec + Send + 'static,
    {
        let (mut t_read, mut t_write) = transport.into_split();
        let post_office = Arc::new(PostOffice::default());
        let weak_post_office = Arc::downgrade(&post_office);

        // Start a task that continually checks for responses and delivers them using the
        // post office
        let response_task = tokio::spawn(async move {
            loop {
                match t_read.receive::<Response<U>>().await {
                    Ok(Some(res)) => {
                        // Try to send response to appropriate mailbox
                        // TODO: How should we handle false response? Did logging in past
                        post_office.deliver_response(res).await;
                    }
                    Ok(None) => {
                        break;
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
        });

        let (tx, mut rx) = mpsc::channel::<Request<T>>(1);
        let request_task = tokio::spawn(async move {
            while let Some(req) = rx.recv().await {
                if t_write.send(req).await.is_err() {
                    break;
                }
            }
        });

        let channel = Channel {
            tx,
            post_office: weak_post_office,
        };

        Ok(Self {
            channel,
            request_task,
            response_task,
            _response_type: PhantomData,
        })
    }
}

impl<T, U> Session<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    /// Waits for the session to terminate, which results when the receiving end of the network
    /// connection is closed (or the session is shutdown)
    pub async fn wait(self) -> Result<(), JoinError> {
        tokio::try_join!(self.request_task, self.response_task).map(|_| ())
    }

    /// Abort the session's current connection by forcing its tasks to abort
    pub fn abort(&self) {
        self.request_task.abort();
        self.response_task.abort();
    }

    /// Clones the underlying channel for requests and returns the cloned instance
    pub fn clone_channel(&self) -> Channel<T, U> {
        self.channel.clone()
    }
}

impl<T, U> Deref for Session<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    type Target = Channel<T, U>;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

impl<T, U> DerefMut for Session<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.channel
    }
}

impl<T, U> From<Session<T, U>> for Channel<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    fn from(session: Session<T, U>) -> Self {
        session.channel
    }
}
