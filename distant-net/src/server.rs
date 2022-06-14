use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::io;

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

#[async_trait]
pub trait Server: Send {
    /// Type of data received by the server
    type Request: DeserializeOwned + Send + Sync;

    /// Type of data sent back by the server
    type Response: Serialize + Send;

    /// Type of data to store globally in the server's state
    type GlobalData: Default + Send + Sync;

    /// Type of data to store locally tied to the specific connection
    type LocalData: Default + Send + Sync;

    /// Invoked upon receiving a request from a client. The server should process this
    /// request, which can be found in `ctx`, and send one or more replies in response.
    async fn on_request(
        ctx: &ServerCtx<Self::Request, Self::Response, Self::GlobalData, Self::LocalData>,
    ) -> io::Result<()>;

    /// When an error occurs within the server while it is handling a request, this function will
    /// be invoked. This can be used to provide user-level error reporting back to the client,
    /// or used to discard an error and ignore the client. By default, this function will
    /// discard all errors and send nothing back to the client.
    ///
    /// Note that this handler is not allowed to fail, so any error that occurs within it needs
    /// to either be swallowed or at best reported within the server in some other way.
    #[allow(unused_variables)]
    async fn on_error_with_request(
        ctx: &ServerCtx<Self::Request, Self::Response, Self::GlobalData, Self::LocalData>,
        err: io::Error,
    ) {
        // Do nothing!
    }
}
