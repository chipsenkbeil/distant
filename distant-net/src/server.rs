use crate::{BoxedCodec, FramedTransport};
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

    /// Invoked to facilitate a handshake between server and client upon establishing a connection,
    /// returning an updated [`FramedTransport`] once the handshake is complete
    async fn on_handshake<T: Send>(
        &self,
        transport: FramedTransport<T, BoxedCodec>,
    ) -> io::Result<FramedTransport<T, BoxedCodec>> {
        Ok(transport)
    }

    /// Invoked upon a new connection becoming established, which provides a mutable reference to
    /// the data created for the connection. This can be useful in performing some additional
    /// initialization on the data prior to it being used anywhere else.
    #[allow(unused_variables)]
    async fn on_accept(&self, local_data: &mut Self::LocalData) {}

    /// Invoked upon receiving a request from a client. The server should process this
    /// request, which can be found in `ctx`, and send one or more replies in response.
    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>);
}
