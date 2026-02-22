use std::fmt;

use super::ServerReply;
use crate::net::common::{ConnectionId, Request};

/// Represents contextual information for working with an inbound request.
pub struct RequestCtx<T, U> {
    /// Unique identifer associated with the connection that sent the request.
    pub connection_id: ConnectionId,

    /// The request being handled.
    pub request: Request<T>,

    /// Used to send replies back to be sent out by the server.
    pub reply: ServerReply<U>,
}

impl<T, U> fmt::Debug for RequestCtx<T, U>
where
    T: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestCtx")
            .field("connection_id", &self.connection_id)
            .field("request", &self.request)
            .field("reply", &"...")
            .finish()
    }
}
