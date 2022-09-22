use crate::{ConnectionId, Request, ServerReply};
use std::sync::Arc;

/// Represents contextual information for working with an inbound request
pub struct ServerCtx<T, U, D> {
    /// Unique identifer associated with the connection that sent the request
    pub connection_id: ConnectionId,

    /// The request being handled
    pub request: Request<T>,

    /// Used to send replies back to be sent out by the server
    pub reply: ServerReply<U>,

    /// Reference to the connection's local data
    pub local_data: Arc<D>,
}

/// Represents contextual information for working with an inbound connection
pub struct ConnectionCtx<'a, A, D> {
    /// Unique identifer associated with the connection
    pub connection_id: ConnectionId,

    /// Authenticator to use to issue challenges to the connection to ensure it is valid
    pub authenticator: &'a mut A,

    /// Reference to the connection's local data
    pub local_data: &'a mut D,
}
