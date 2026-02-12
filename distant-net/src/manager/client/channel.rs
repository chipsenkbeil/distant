use std::io;
use std::ops::{Deref, DerefMut};

use log::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::task::JoinHandle;

use crate::client::{Client, ClientConfig, UntypedClient};
use crate::common::{ConnectionId, FramedTransport, InmemoryTransport, UntypedRequest};
use crate::manager::data::{ManagerRequest, ManagerResponse};

/// Represents a raw channel between a manager client and server. Underneath, this routes incoming
/// and outgoing data from a proxied server to an inmemory transport.
pub struct RawChannel {
    transport: FramedTransport<InmemoryTransport>,
    task: JoinHandle<()>,
}

impl RawChannel {
    pub fn abort(&self) {
        self.task.abort();
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
            self.transport,
            ClientConfig::default().with_maximum_silence_duration(),
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
            self.transport,
            ClientConfig::default().with_maximum_silence_duration(),
        )
    }

    /// Returns reference to the underlying framed transport.
    pub fn as_framed_transport(&self) -> &FramedTransport<InmemoryTransport> {
        &self.transport
    }

    /// Returns mutable reference to the underlying framed transport.
    pub fn as_mut_framed_transport(&mut self) -> &mut FramedTransport<InmemoryTransport> {
        &mut self.transport
    }

    /// Consumes the channel, returning the underlying framed transport.
    pub fn into_framed_transport(self) -> FramedTransport<InmemoryTransport> {
        self.transport
    }
}

impl Deref for RawChannel {
    type Target = FramedTransport<InmemoryTransport>;

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
                ManagerResponse::Error { description } => Err(io::Error::other(description)),
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
        let (mut proxy, transport) = FramedTransport::pair(1);

        let mut manager_channel = client.clone_channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    maybe_response = mailbox.next() => {
                        if maybe_response.is_none() {
                            debug!("[Conn {connection_id} :: Chan {channel_id}] Closing from no more responses");
                            break;
                        }

                        match maybe_response.unwrap().payload {
                            ManagerResponse::Channel { response, .. } => {
                                if let Err(x) = proxy.write_frame(response.to_bytes()).await {
                                    error!(
                                        "[Conn {connection_id} :: Chan {channel_id}] Write response failed: {x}"
                                    );
                                }
                            }
                            ManagerResponse::ChannelClosed { .. } => {
                                break;
                            }
                            _ => continue,
                        }
                    }
                    result = proxy.read_frame() => {
                        match result {
                            Ok(Some(frame)) => {
                                let request = match UntypedRequest::from_slice(frame.as_item()) {
                                    Ok(x) => x.into_owned(),
                                    Err(x) => {
                                        error!("[Conn {connection_id} :: Chan {channel_id}] Parse request failed: {x}");
                                        continue;
                                    }
                                };

                                // NOTE: In this situation, we do not expect a response to this
                                //       request (even if the server sends something back)
                                if let Err(x) = manager_channel
                                    .fire(ManagerRequest::Channel {
                                        id: channel_id,
                                        request,
                                    })
                                    .await
                                {
                                    error!("[Conn {connection_id} :: Chan {channel_id}] Forward failed: {x}");
                                }
                            }
                            Ok(None) => {
                                debug!("[Conn {connection_id} :: Chan {channel_id}] Closing from no more requests");
                                break;
                            }
                            Err(x) => {
                                error!("[Conn {connection_id} :: Chan {channel_id}] Read frame failed: {x}");
                            }
                        }
                    }
                }
            }
        });

        Ok(RawChannel { transport, task })
    }
}
