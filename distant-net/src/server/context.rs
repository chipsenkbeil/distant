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
