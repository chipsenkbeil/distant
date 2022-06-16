use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};

mod connection;
pub use connection::*;

mod context;
pub use context::*;

mod ext;
pub use ext::*;

mod r#ref;
pub use r#ref::*;

mod reply;
pub use reply::*;

mod state;
pub use state::*;

/// Interface for a general-purpose server that receives requests to handle
#[async_trait]
pub trait Server: Send {
    /// Type of data received by the server
    type Request: DeserializeOwned + Send + Sync;

    /// Type of data sent back by the server
    type Response: Serialize + Send;

    /// Type of data to store locally tied to the specific connection
    type LocalData: Default + Send + Sync;

    /// Invoked upon receiving a request from a client. The server should process this
    /// request, which can be found in `ctx`, and send one or more replies in response.
    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>);
}
