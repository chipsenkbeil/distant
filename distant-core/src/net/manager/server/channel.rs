use std::io;

use log::*;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::net::client::ClientConfig;
use crate::net::common::{FramedTransport, InmemoryTransport, Response, UntypedRequest};
use crate::net::manager::data::ManagerResponse;
use crate::net::server::ServerReply;

use super::connection::{ManagerChannel, ManagerConnection};

/// Server-side raw channel that creates a distant protocol [`Channel`](crate::Channel)
/// backed by an internal [`ManagerChannel`] on a [`ManagerConnection`].
///
/// This allows the manager server itself to issue distant protocol requests
/// (e.g. `TunnelOpen`) against a managed connection without going through an
/// external client transport.
pub struct InternalRawChannel {
    transport: FramedTransport<InmemoryTransport>,
    _proxy_task: JoinHandle<()>,
    manager_channel: ManagerChannel,
}

impl InternalRawChannel {
    /// Opens an internal channel on the given connection.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection's action channel is closed.
    pub fn open(connection: &ManagerConnection) -> io::Result<Self> {
        let (response_tx, mut response_rx) = mpsc::unbounded_channel::<Response<ManagerResponse>>();
        let reply = ServerReply {
            origin_id: format!("internal_{}", rand::random::<u32>()),
            tx: response_tx,
        };

        let manager_channel = connection.open_channel(reply)?;

        let (mut proxy, transport) = FramedTransport::pair(1);

        let channel_for_proxy = manager_channel.clone();
        let proxy_task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    maybe_response = response_rx.recv() => {
                        match maybe_response {
                            Some(response) => match response.payload {
                                ManagerResponse::Channel { response, .. } => {
                                    if let Err(e) = proxy.write_frame(response.to_bytes()).await {
                                        debug!("Internal channel write failed: {e}");
                                        break;
                                    }
                                }
                                ManagerResponse::ChannelClosed { .. } => break,
                                _ => continue,
                            },
                            None => break,
                        }
                    }
                    result = proxy.read_frame() => {
                        match result {
                            Ok(Some(frame)) => {
                                let request = match UntypedRequest::from_slice(frame.as_item()) {
                                    Ok(x) => x.into_owned(),
                                    Err(e) => {
                                        error!("Internal channel parse request failed: {e}");
                                        continue;
                                    }
                                };
                                if let Err(e) = channel_for_proxy.send(request) {
                                    debug!("Internal channel send failed: {e}");
                                    break;
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                debug!("Internal channel read frame failed: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            transport,
            _proxy_task: proxy_task,
            manager_channel,
        })
    }

    /// Consumes this internal channel, returning a typed distant protocol
    /// [`Channel`](crate::Channel) and the underlying [`ManagerChannel`] for
    /// cleanup when the tunnel is stopped.
    pub fn into_parts(self) -> (crate::Channel, ManagerChannel) {
        let client: crate::Client = crate::net::Client::spawn_inmemory(
            self.transport,
            ClientConfig::default().with_maximum_silence_duration(),
        );
        (client.into_channel(), self.manager_channel)
    }
}
