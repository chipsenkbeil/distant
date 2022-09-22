use crate::auth::Authenticator;
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::io;

mod config;
pub use config::*;

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
    type LocalData: Send + Sync;

    /// Returns configuration tied to server instance
    fn config(&self) -> ServerConfig {
        ServerConfig::default()
    }

    /// Invoked upon a new connection becoming established.
    ///
    /// ### Note
    ///
    /// This can be useful in performing some additional initialization on the connection's local
    /// data prior to it being used anywhere else.
    ///
    /// Additionally, the context contains an authenticator which can be used to issue challenges
    /// to the connection to validate its access.
    async fn on_accept<A: Authenticator>(
        &self,
        ctx: ConnectionCtx<'_, A, Self::LocalData>,
    ) -> io::Result<()>;

    /// Invoked upon receiving a request from a client. The server should process this
    /// request, which can be found in `ctx`, and send one or more replies in response.
    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>);
}
