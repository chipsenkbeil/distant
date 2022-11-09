use crate::{
    common::{
        ConnectionId, Destination, FramedTransport, Interest, Map, Transport, UntypedRequest,
        UntypedResponse,
    },
    manager::data::{ManagerChannelId, ManagerResponse},
    server::ServerReply,
};
use log::*;
use serde::Serialize;
use std::{collections::HashMap, io, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle};

/// Represents a connection a distant manager has with some distant-compatible server
pub struct ManagerConnection {
    pub id: ConnectionId,
    pub destination: Destination,
    pub options: Map,
    tx: mpsc::UnboundedSender<Action>,
    transport_task: JoinHandle<()>,
    action_task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct ManagerChannel {
    channel_id: ManagerChannelId,
    tx: mpsc::UnboundedSender<Action>,
}

impl ManagerChannel {
    pub fn id(&self) -> ManagerChannelId {
        self.channel_id
    }

    pub fn send<T: Serialize>(&self, data: Vec<u8>) -> io::Result<()> {
        let channel_id = self.channel_id;
        self.tx
            .send(Action::Write {
                id: channel_id,
                data,
            })
            .map_err(|x| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("channel {channel_id} send failed: {x}"),
                )
            })
    }

    pub fn close(&self) -> io::Result<()> {
        let channel_id = self.channel_id;
        self.tx
            .send(Action::Unregister { id: channel_id })
            .map_err(|x| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("channel {channel_id} close failed: {x}"),
                )
            })
    }
}

impl ManagerConnection {
    pub fn new<T: Transport + 'static>(
        destination: Destination,
        options: Map,
        transport: FramedTransport<T>,
    ) -> Self {
        let connection_id = rand::random();
        let (tx, mut rx) = mpsc::unbounded_channel();

        let (outgoing_tx, outgoing_rx) = mpsc::unbounded_channel();
        let transport_task = tokio::spawn(transport_task(
            connection_id,
            transport,
            outgoing_rx,
            tx.clone(),
            Duration::from_millis(50),
        ));
        let action_task = tokio::spawn(action_task(connection_id, rx, outgoing_tx));

        Self {
            id: connection_id,
            destination,
            options,
            tx,
            transport_task,
            action_task,
        }
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
}

impl Drop for ManagerConnection {
    fn drop(&mut self) {
        self.transport_task.abort();
        self.action_task.abort();
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

    Read {
        data: Vec<u8>,
    },

    Write {
        id: ManagerChannelId,
        data: Vec<u8>,
    },
}

/// Internal task to read and write from a [`Transport`].
///
/// * `id` - the id of the connection.
/// * `transport` - the fully-authenticated transport.
/// * `rx` - used to receive outgoing data to send through the connection.
/// * `tx` - used to send new [`Action`]s to process.
async fn transport_task<T: Transport>(
    id: ConnectionId,
    mut transport: FramedTransport<T>,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
    tx: mpsc::UnboundedSender<Action>,
    sleep_duration: Duration,
) {
    loop {
        let ready = match transport
            .ready(Interest::READABLE | Interest::WRITABLE)
            .await
        {
            Ok(ready) => ready,
            Err(x) => {
                error!("[Conn {id}] Querying ready status failed: {x}");
                break;
            }
        };

        // Keep track of whether we read or wrote anything
        let mut read_blocked = !ready.is_readable();
        let mut write_blocked = !ready.is_writable();

        // If transport is readable, attempt to read a frame and forward it to our action task
        if ready.is_readable() {
            match transport.try_read_frame() {
                Ok(Some(frame)) => {
                    if let Err(x) = tx.send(Action::Read {
                        data: frame.into_item().into_owned(),
                    }) {
                        error!("[Conn {id}] Failed to forward frame: {x}");
                    }
                }
                Ok(None) => {
                    debug!("[Conn {id}] Connection closed");
                    break;
                }
                Err(x) if x.kind() == io::ErrorKind::WouldBlock => read_blocked = true,
                Err(x) => {
                    error!("[Conn {id}] {x}");
                }
            }
        }

        // If transport is writable, check if we have something to write
        if ready.is_writable() {
            if let Ok(data) = rx.try_recv() {
                match transport.try_write_frame(data) {
                    Ok(()) => (),
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => write_blocked = true,
                    Err(x) => error!("[Conn {id}] Send failed: {x}"),
                }
            } else {
                // In the case of flushing, there are two scenarios in which we want to
                // mark no write occurring:
                //
                // 1. When flush did not write any bytes, which can happen when the buffer
                //    is empty
                // 2. When the call to write bytes blocks
                match transport.try_flush() {
                    Ok(0) => write_blocked = true,
                    Ok(_) => (),
                    Err(x) if x.kind() == io::ErrorKind::WouldBlock => write_blocked = true,
                    Err(x) => {
                        error!("[Conn {id}] {x}");
                    }
                }
            }
        }

        // If we did not read or write anything, sleep a bit to offload CPU usage
        if read_blocked && write_blocked {
            tokio::time::sleep(sleep_duration).await;
        }
    }
}

/// Internal task to process [`Action`] items.
///
/// * `id` - the id of the connection.
/// * `rx` - used to receive new [`Action`]s to process.
/// * `tx` - used to send outgoing data through the connection.
async fn action_task(
    id: ConnectionId,
    mut rx: mpsc::UnboundedReceiver<Action>,
    tx: mpsc::UnboundedSender<Vec<u8>>,
) {
    let mut registered = HashMap::new();

    while let Some(action) = rx.recv().await {
        match action {
            Action::Register { id, reply } => {
                registered.insert(id, reply);
            }
            Action::Unregister { id } => {
                registered.remove(&id);
            }
            Action::Read { data } => {
                // Partially parse data into a request so we can modify the origin id
                let mut response = match UntypedResponse::from_slice(&data) {
                    Ok(response) => response,
                    Err(x) => {
                        error!("[Conn {id}] Failed to parse response during read: {x}");
                        continue;
                    }
                };

                // Split {channel id}_{request id} back into pieces and
                // update the origin id to match the request id only
                let channel_id = match response.origin_id.split_once('_') {
                    Some((cid_str, oid_str)) => {
                        if let Ok(cid) = cid_str.parse::<ManagerChannelId>() {
                            response.set_origin_id(oid_str.to_string());
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
                        data: response.to_bytes(),
                    };
                    if let Err(x) = reply.send(response).await {
                        error!("[Conn {id}] {x}");
                    }
                }
            }
            Action::Write { id, data } => {
                // Partially parse data into a request so we can modify the id
                let mut request = match UntypedRequest::from_slice(&data) {
                    Ok(request) => request,
                    Err(x) => {
                        error!("[Conn {id}] Failed to parse request during write: {x}");
                        continue;
                    }
                };

                // Combine channel id with request id so we can properly forward
                // the response containing this in the origin id
                request.set_id(format!("{id}_{}", request.id));

                if let Err(x) = tx.send(request.to_bytes()) {
                    error!("[Conn {id}] {x}");
                }
            }
        }
    }
}
