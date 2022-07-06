use crate::{
    manager::{
        data::{ChannelId, ConnectionId, Destination, Extra},
        BoxedDistantReader, BoxedDistantWriter,
    },
    DistantMsg, DistantRequestData, DistantResponseData, ManagerResponse,
};
use distant_net::{Request, Response, ServerReply};
use log::*;
use std::{collections::HashMap, io};
use tokio::{sync::mpsc, task::JoinHandle};

/// Represents a connection a distant manager has with some distant-compatible server
pub struct DistantManagerConnection {
    pub id: ConnectionId,
    pub destination: Destination,
    pub extra: Extra,
    tx: mpsc::Sender<StateMachine>,
    reader_task: JoinHandle<()>,
    writer_task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct DistantManagerChannel {
    channel_id: ChannelId,
    tx: mpsc::Sender<StateMachine>,
}

impl DistantManagerChannel {
    pub fn id(&self) -> ChannelId {
        self.channel_id
    }

    pub async fn send(&self, request: Request<DistantMsg<DistantRequestData>>) -> io::Result<()> {
        let channel_id = self.channel_id;
        self.tx
            .send(StateMachine::Write {
                id: channel_id,
                request,
            })
            .await
            .map_err(|x| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("channel {} send failed: {}", channel_id, x),
                )
            })
    }

    pub async fn close(&self) -> io::Result<()> {
        let channel_id = self.channel_id;
        self.tx
            .send(StateMachine::Unregister { id: channel_id })
            .await
            .map_err(|x| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("channel {} close failed: {}", channel_id, x),
                )
            })
    }
}

enum StateMachine {
    Register {
        id: ChannelId,
        reply: ServerReply<ManagerResponse>,
    },

    Unregister {
        id: ChannelId,
    },

    Read {
        response: Response<DistantMsg<DistantResponseData>>,
    },

    Write {
        id: ChannelId,
        request: Request<DistantMsg<DistantRequestData>>,
    },
}

impl DistantManagerConnection {
    pub fn new(
        destination: Destination,
        extra: Extra,
        mut writer: BoxedDistantWriter,
        mut reader: BoxedDistantReader,
    ) -> Self {
        let connection_id = rand::random();
        let (tx, mut rx) = mpsc::channel(1);
        let reader_task = {
            let tx = tx.clone();
            tokio::spawn(async move {
                loop {
                    match reader.read().await {
                        Ok(Some(response)) => {
                            if tx.send(StateMachine::Read { response }).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(x) => {
                            error!("[Conn {}] {}", connection_id, x);
                            continue;
                        }
                    }
                }
            })
        };
        let writer_task = tokio::spawn(async move {
            let mut registered = HashMap::new();
            while let Some(state_machine) = rx.recv().await {
                match state_machine {
                    StateMachine::Register { id, reply } => {
                        registered.insert(id, reply);
                    }
                    StateMachine::Unregister { id } => {
                        registered.remove(&id);
                    }
                    StateMachine::Read { mut response } => {
                        // Split {channel id}_{request id} back into pieces and
                        // update the origin id to match the request id only
                        let channel_id = match response.origin_id.split_once('_') {
                            Some((cid_str, oid_str)) => {
                                if let Ok(cid) = cid_str.parse::<ChannelId>() {
                                    response.origin_id = oid_str.to_string();
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
                                response,
                            };
                            if let Err(x) = reply.send(response).await {
                                error!("[Conn {}] {}", connection_id, x);
                            }
                        }
                    }
                    StateMachine::Write { id, request } => {
                        // Combine channel id with request id so we can properly forward
                        // the response containing this in the origin id
                        let request = Request {
                            id: format!("{}_{}", id, request.id),
                            payload: request.payload,
                        };
                        if let Err(x) = writer.write(request).await {
                            error!("[Conn {}] {}", connection_id, x);
                        }
                    }
                }
            }
        });

        Self {
            id: connection_id,
            destination,
            extra,
            tx,
            reader_task,
            writer_task,
        }
    }

    pub async fn open_channel(
        &self,
        reply: ServerReply<ManagerResponse>,
    ) -> io::Result<DistantManagerChannel> {
        let channel_id = rand::random();
        let _ = self
            .tx
            .send(StateMachine::Register {
                id: channel_id,
                reply,
            })
            .await
            .map_err(|x| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("open_channel failed: {}", x),
                )
            })?;
        Ok(DistantManagerChannel {
            channel_id,
            tx: self.tx.clone(),
        })
    }
}

impl Drop for DistantManagerConnection {
    fn drop(&mut self) {
        self.reader_task.abort();
        self.writer_task.abort();
    }
}
