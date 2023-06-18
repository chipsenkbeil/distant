use super::ServerReply;
use crate::common::{ConnectionId, Request};

/// Represents contextual information for working with an inbound request.
pub struct RequestCtx<T, U> {
    /// Unique identifer associated with the connection that sent the request.
    pub connection_id: ConnectionId,

    /// The request being handled.
    pub request: Request<T>,

    /// Used to send replies back to be sent out by the server.
    pub reply: ServerReply<U>,
}
