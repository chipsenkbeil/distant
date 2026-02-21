use std::collections::HashMap;
use std::{fmt, io};

use log::*;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::net::client::{Mailbox, UntypedClient};
use crate::net::common::{ConnectionId, Destination, Map, UntypedRequest, UntypedResponse};
use crate::net::manager::data::{ManagerChannelId, ManagerResponse};
use crate::net::server::ServerReply;

/// Represents a connection a distant manager has with some distant-compatible server
pub struct ManagerConnection {
    pub id: ConnectionId,
    pub destination: Destination,
    pub options: Map,
    tx: mpsc::UnboundedSender<Action>,

    action_task: JoinHandle<()>,
    request_task: JoinHandle<()>,
    response_task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct ManagerChannel {
    channel_id: ManagerChannelId,
    tx: mpsc::UnboundedSender<Action>,
}

impl ManagerChannel {
    /// Returns the id associated with the channel.
    pub fn id(&self) -> ManagerChannelId {
        self.channel_id
    }

    /// Sends the untyped request to the server on the other side of the channel.
    pub fn send(&self, req: UntypedRequest<'static>) -> io::Result<()> {
        let id = self.channel_id;

        self.tx.send(Action::Write { id, req }).map_err(|x| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("channel {id} send failed: {x}"),
            )
        })
    }

    /// Closes the channel, unregistering it with the connection.
    pub fn close(&self) -> io::Result<()> {
        let id = self.channel_id;
        self.tx.send(Action::Unregister { id }).map_err(|x| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("channel {id} close failed: {x}"),
            )
        })
    }
}

impl ManagerConnection {
    pub async fn spawn(
        spawn: Destination,
        options: Map,
        mut client: UntypedClient,
    ) -> io::Result<Self> {
        let connection_id = rand::random();
        let (tx, rx) = mpsc::unbounded_channel();

        // NOTE: Ensure that the connection is severed when the client is dropped; otherwise, when
        // the connection is terminated via aborting it or the connection being dropped, the
        // connection will persist which can cause problems such as lonely shutdown of the server
        // never triggering!
        client.shutdown_on_drop(true);

        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let action_task = tokio::spawn(action_task(connection_id, rx, request_tx));
        let response_task = tokio::spawn(response_task(
            connection_id,
            client.assign_default_mailbox(100).await?,
            tx.clone(),
        ));
        let request_task = tokio::spawn(request_task(connection_id, client, request_rx));

        Ok(Self {
            id: connection_id,
            destination: spawn,
            options,
            tx,
            action_task,
            request_task,
            response_task,
        })
    }

    pub fn open_channel(&self, reply: ServerReply<ManagerResponse>) -> io::Result<ManagerChannel> {
        let channel_id = rand::random();
        self.tx
            .send(Action::Register {
                id: channel_id,
                reply,
            })
            .map_err(|x| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("open_channel failed: {x}"),
                )
            })?;
        Ok(ManagerChannel {
            channel_id,
            tx: self.tx.clone(),
        })
    }

    pub async fn channel_ids(&self) -> io::Result<Vec<ManagerChannelId>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(Action::GetRegistered { cb: tx })
            .map_err(|x| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("channel_ids failed: {x}"),
                )
            })?;

        let channel_ids = rx.await.map_err(|x| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                format!("channel_ids callback dropped: {x}"),
            )
        })?;
        Ok(channel_ids)
    }

    /// Aborts the tasks used to engage with the connection.
    pub fn abort(&self) {
        self.action_task.abort();
        self.request_task.abort();
        self.response_task.abort();
    }
}

impl Drop for ManagerConnection {
    fn drop(&mut self) {
        self.abort();
    }
}

enum Action {
    Register {
        id: ManagerChannelId,
        reply: ServerReply<ManagerResponse>,
    },

    Unregister {
        id: ManagerChannelId,
    },

    GetRegistered {
        cb: oneshot::Sender<Vec<ManagerChannelId>>,
    },

    Read {
        res: UntypedResponse<'static>,
    },

    Write {
        id: ManagerChannelId,
        req: UntypedRequest<'static>,
    },
}

impl fmt::Debug for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Register { id, .. } => write!(f, "Action::Register {{ id: {id}, .. }}"),
            Self::Unregister { id } => write!(f, "Action::Unregister {{ id: {id} }}"),
            Self::GetRegistered { .. } => write!(f, "Action::GetRegistered {{ .. }}"),
            Self::Read { .. } => write!(f, "Action::Read {{ .. }}"),
            Self::Write { id, .. } => write!(f, "Action::Write {{ id: {id}, .. }}"),
        }
    }
}

/// Internal task to process outgoing [`UntypedRequest`]s.
async fn request_task(
    id: ConnectionId,
    mut client: UntypedClient,
    mut rx: mpsc::UnboundedReceiver<UntypedRequest<'static>>,
) {
    while let Some(req) = rx.recv().await {
        trace!("[Conn {id}] Firing off request {}", req.id);
        if let Err(x) = client.fire(req).await {
            error!("[Conn {id}] Failed to send request: {x}");
        }
    }

    trace!("[Conn {id}] Manager request task closed");
}

/// Internal task to process incoming [`UntypedResponse`]s.
async fn response_task(
    id: ConnectionId,
    mut mailbox: Mailbox<UntypedResponse<'static>>,
    tx: mpsc::UnboundedSender<Action>,
) {
    while let Some(res) = mailbox.next().await {
        trace!(
            "[Conn {id}] Receiving response {} to request {}",
            res.id,
            res.origin_id
        );
        if let Err(x) = tx.send(Action::Read { res }) {
            error!("[Conn {id}] Failed to forward received response: {x}");
        }
    }

    trace!("[Conn {id}] Manager response task closed");
}

/// Internal task to process [`Action`] items.
///
/// * `id` - the id of the connection.
/// * `rx` - used to receive new [`Action`]s to process.
/// * `tx` - used to send outgoing requests through the connection.
async fn action_task(
    id: ConnectionId,
    mut rx: mpsc::UnboundedReceiver<Action>,
    tx: mpsc::UnboundedSender<UntypedRequest<'static>>,
) {
    let mut registered = HashMap::new();

    while let Some(action) = rx.recv().await {
        trace!("[Conn {id}] {action:?}");

        match action {
            Action::Register { id, reply } => {
                registered.insert(id, reply);
            }
            Action::Unregister { id } => {
                registered.remove(&id);
            }
            Action::GetRegistered { cb } => {
                let _ = cb.send(registered.keys().copied().collect());
            }
            Action::Read { mut res } => {
                // Split {channel id}_{request id} back into pieces and
                // update the origin id to match the request id only
                let channel_id = match res.origin_id.split_once('_') {
                    Some((cid_str, oid_str)) => {
                        if let Ok(cid) = cid_str.parse::<ManagerChannelId>() {
                            res.set_origin_id(oid_str.to_string());
                            cid
                        } else {
                            continue;
                        }
                    }
                    None => continue,
                };

                if let Some(reply) = registered.get(&channel_id) {
                    let response = ManagerResponse::Channel {
                        id: channel_id,
                        response: res,
                    };

                    if let Err(x) = reply.send(response) {
                        error!("[Conn {id}] {x}");
                    }
                }
            }
            Action::Write { id, mut req } => {
                // Combine channel id with request id so we can properly forward
                // the response containing this in the origin id
                req.set_id(format!("{id}_{}", req.id));

                if let Err(x) = tx.send(req) {
                    error!("[Conn {id}] {x}");
                }
            }
        }
    }

    trace!("[Conn {id}] Manager action task closed");
}
