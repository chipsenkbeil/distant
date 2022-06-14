use crate::{Id, Request, Response, ServerState};
use std::{io, sync::Arc};
use tokio::sync::mpsc;

/// Represents contextual information for working with an inbound request
pub struct ServerRequestCtx<RequestData, ResponseData, GlobalData, LocalData> {
    /// Unique identifer associated with the connection that sent the request
    pub connection_id: Id,

    /// The request being handled
    pub request: Request<RequestData>,

    /// Used to send replies back to be sent out by the server
    pub reply: ServerCtxReply<ResponseData>,

    /// Reference to the server's state
    pub state: Arc<ServerState<GlobalData, LocalData>>,
}

/// Utility to send ad-hoc replies from the server back through the connection
pub struct ServerCtxReply<T> {
    pub(crate) origin_id: Id,
    pub(crate) tx: mpsc::Sender<Response<T>>,
}

impl<T> Clone for ServerCtxReply<T> {
    fn clone(&self) -> Self {
        Self {
            origin_id: self.origin_id,
            tx: self.tx.clone(),
        }
    }
}

impl<T> ServerCtxReply<T> {
    pub async fn send(&self, data: T) -> io::Result<()> {
        self.tx
            .send(Response::new(self.origin_id, data))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Connection reply closed"))
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}
