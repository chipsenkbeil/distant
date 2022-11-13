use crate::{
    client::{Client, ReconnectStrategy, UntypedClient},
    common::{ConnectionId, FramedTransport, InmemoryTransport},
    manager::data::{ManagerRequest, ManagerResponse},
};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    io,
    ops::{Deref, DerefMut},
};
use tokio::{sync::oneshot, task::JoinHandle};

/// Represents a raw channel between a manager client and server. Underneath, this routes incoming
/// and outgoing data from a proxied server to an inmemory transport.
pub struct RawChannel {
    transport: InmemoryTransport,
    forward_task: JoinHandle<()>,
    mailbox_task: JoinHandle<()>,
}

impl RawChannel {
    pub fn abort(&self) {
        self.forward_task.abort();
        self.mailbox_task.abort();
    }

    /// Consumes this channel, returning a typed client wrapping the transport.
    ///
    /// ### Note
    ///
    /// This does not perform any additional handshakes or authentication. All authentication was
    /// performed during separate connection and this merely wraps an inmemory transport that maps
    /// to the primary connection.
    pub fn into_client<T, U>(self) -> Client<T, U>
    where
        T: Send + Sync + Serialize + 'static,
        U: Send + Sync + DeserializeOwned + 'static,
    {
        Client::spawn_inmemory(
            FramedTransport::plain(self.transport),
            ReconnectStrategy::Fail,
        )
    }

    /// Consumes this channel, returning an untyped client wrapping the transport.
    ///
    /// ### Note
    ///
    /// This does not perform any additional handshakes or authentication. All authentication was
    /// performed during separate connection and this merely wraps an inmemory transport that maps
    /// to the primary connection.
    pub fn into_untyped_client(self) -> UntypedClient {
        UntypedClient::spawn_inmemory(
            FramedTransport::plain(self.transport),
            ReconnectStrategy::Fail,
        )
    }
}

impl Deref for RawChannel {
    type Target = InmemoryTransport;

    fn deref(&self) -> &Self::Target {
        &self.transport
    }
}

impl DerefMut for RawChannel {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.transport
    }
}

impl RawChannel {
    pub(super) async fn spawn(
        connection_id: ConnectionId,
        client: &mut Client<ManagerRequest, ManagerResponse>,
    ) -> io::Result<Self> {
        let mut mailbox = client
            .mail(ManagerRequest::OpenChannel { id: connection_id })
            .await?;

        // Wait for the first response, which should be channel confirmation
        let channel_id = match mailbox.next().await {
            Some(response) => match response.payload {
                ManagerResponse::ChannelOpened { id } => Ok(id),
                ManagerResponse::Error { description } => {
                    Err(io::Error::new(io::ErrorKind::Other, description))
                }
                x => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("[Conn {connection_id}] Raw channel open unexpected response: {x:?}"),
                )),
            },
            None => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                format!("[Conn {connection_id}] Raw channel mailbox aborted"),
            )),
        }?;

        // Spawn our channel proxy transport
        let (tx, mut rx, transport) = InmemoryTransport::make(1);
        let (channel_close_tx, mut channel_close_rx) = oneshot::channel();
        let mailbox_task = tokio::spawn(async move {
            while let Some(response) = mailbox.next().await {
                match response.payload {
                    ManagerResponse::Channel { data, .. } => {
                        if let Err(x) = tx.send(data).await {
                            error!("[Conn {connection_id} :: Chan {channel_id}] {x}");
                        }
                    }
                    ManagerResponse::ChannelClosed { .. } => {
                        let _ = channel_close_tx.send(());
                        break;
                    }
                    _ => continue,
                }
            }
        });

        let mut manager_channel = client.clone_channel();
        let forward_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut channel_close_rx => { break }
                    data = rx.recv() => {
                        match data {
                            Some(data) => {
                                // NOTE: In this situation, we do not expect a response to this
                                //       request (even if the server sends something back)
                                if let Err(x) = manager_channel
                                    .fire(ManagerRequest::Channel {
                                        id: channel_id,
                                        data,
                                    })
                                    .await
                                {
                                    error!("[Conn {connection_id} :: Chan {channel_id}] {x}");
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        });

        Ok(RawChannel {
            transport,
            forward_task,
            mailbox_task,
        })
    }
}
