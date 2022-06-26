use crate::{Request, ServerReply};
use std::sync::Arc;

/// Represents contextual information for working with an inbound request
pub struct ServerCtx<RequestData, ResponseData, LocalData> {
    /// Unique identifer associated with the connection that sent the request
    pub connection_id: usize,

    /// The request being handled
    pub request: Request<RequestData>,

    /// Used to send replies back to be sent out by the server
    pub reply: ServerReply<ResponseData>,

    /// Reference to the connection's local data
    pub local_data: Arc<LocalData>,
}
