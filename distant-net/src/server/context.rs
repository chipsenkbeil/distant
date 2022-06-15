use crate::{Id, Request, ServerReply, ServerState};
use std::sync::Arc;

/// Represents contextual information for working with an inbound request
pub struct ServerCtx<RequestData, ResponseData, GlobalData, LocalData> {
    /// Unique identifer associated with the connection that sent the request
    pub connection_id: Id,

    /// The request being handled
    pub request: Request<RequestData>,

    /// Used to send replies back to be sent out by the server
    pub reply: ServerReply<ResponseData>,

    /// Reference to the server's state
    pub state: Arc<ServerState<GlobalData, LocalData>>,
}

impl<RequestData, ResponseData, GlobalData, LocalData>
    ServerCtx<RequestData, ResponseData, GlobalData, LocalData>
{
    /// Invokes `f` with a reference to the local data for the connection
    pub async fn with_local_data<T, F>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&LocalData) -> T,
    {
        let id = self.connection_id;
        self.state
            .connections
            .read()
            .await
            .get(&id)
            .map(|connection| f(&connection.data))
    }

    /// Invokes `f` with a mutable reference to the local data for the connection
    pub async fn with_mut_local_data<T, F>(&self, f: F) -> Option<T>
    where
        F: FnOnce(&mut LocalData) -> T,
    {
        let id = self.connection_id;
        self.state
            .connections
            .write()
            .await
            .get_mut(&id)
            .map(|connection| f(&mut connection.data))
    }
}
