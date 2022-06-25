use crate::{
    Codec, FramedTransport, IntoSplit, RawTransport, RawTransportRead, RawTransportWrite, Request,
    Response, TypedAsyncRead, TypedAsyncWrite,
};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};
use tokio::{
    io,
    sync::mpsc,
    task::{JoinError, JoinHandle},
};

mod channel;
pub use channel::*;

mod ext;
pub use ext::*;

/// Represents a client that can be used to send requests & receive responses from a server
pub struct Client<T, U>
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
}

impl<T, U> Client<T, U>
where
    T: Send + Sync + Serialize,
    U: Send + Sync + DeserializeOwned,
{
    /// Initializes a client using the provided reader and writer
    pub fn new<R, W>(mut writer: W, mut reader: R) -> io::Result<Self>
    where
        R: TypedAsyncRead<Response<U>> + Send + 'static,
        W: TypedAsyncWrite<Request<T>> + Send + 'static,
    {
        let post_office = Arc::new(PostOffice::default());
        let weak_post_office = Arc::downgrade(&post_office);

        // Start a task that continually checks for responses and delivers them using the
        // post office
        let response_task = tokio::spawn(async move {
            loop {
                match reader.read().await {
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
                if writer.write(req).await.is_err() {
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
        })
    }

    /// Initializes a client using the provided framed transport
    pub fn from_framed_transport<TR, C>(transport: FramedTransport<TR, C>) -> io::Result<Self>
    where
        TR: RawTransport + IntoSplit + 'static,
        <TR as IntoSplit>::Read: RawTransportRead,
        <TR as IntoSplit>::Write: RawTransportWrite,
        C: Codec + Send + 'static,
    {
        let (writer, reader) = transport.into_split();
        Self::new(writer, reader)
    }

    /// Convert into underlying channel
    pub fn into_channel(self) -> Channel<T, U> {
        self.channel
    }

    /// Clones the underlying channel for requests and returns the cloned instance
    pub fn clone_channel(&self) -> Channel<T, U> {
        self.channel.clone()
    }

    /// Waits for the client to terminate, which results when the receiving end of the network
    /// connection is closed (or the client is shutdown)
    pub async fn wait(self) -> Result<(), JoinError> {
        tokio::try_join!(self.request_task, self.response_task).map(|_| ())
    }

    /// Abort the client's current connection by forcing its tasks to abort
    pub fn abort(&self) {
        self.request_task.abort();
        self.response_task.abort();
    }

    /// Returns true if client's underlying event processing has finished/terminated
    pub fn is_finished(&self) -> bool {
        self.request_task.is_finished() && self.response_task.is_finished()
    }
}

impl<T, U> Deref for Client<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    type Target = Channel<T, U>;

    fn deref(&self) -> &Self::Target {
        &self.channel
    }
}

impl<T, U> DerefMut for Client<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.channel
    }
}

impl<T, U> From<Client<T, U>> for Channel<T, U>
where
    T: Send + Sync + Serialize + 'static,
    U: Send + Sync + DeserializeOwned + 'static,
{
    fn from(client: Client<T, U>) -> Self {
        client.channel
    }
}
