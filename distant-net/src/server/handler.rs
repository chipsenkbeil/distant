use crate::ServerCtx;
use std::{future::Future, io, pin::Pin};

/// Interface to be invoked when new data is received by the server
pub trait ServerHandler {
    /// Type of data received by handler
    type Request;

    /// Type of data sent back by handler
    type Response;

    /// Type of data to store globally in the server's state
    type GlobalData;

    /// Type of data to store locally tied to the specific connection
    type LocalData;

    /// Invoked whenever a new request is received
    #[allow(clippy::type_complexity)]
    fn on_request<'a>(
        &'a self,
        ctx: ServerCtx<Self::Request, Self::Response, Self::GlobalData, Self::LocalData>,
    ) -> Pin<Box<dyn Future<Output = io::Result<Self::Response>> + Send + 'a>>
    where
        Self: Sync + 'a;
}

/// Generates an implementation of a [`ServerHandler`]
///
/// `on_request` can optionally include `this` as a reference to `self`.
///
/// ```ignore
/// on_request: |ctx| {
///     // no way to access &self
/// }
///
/// on_request: |ctx, this| {
///     // this is &self
/// }
/// ```
#[macro_export]
macro_rules! server_handler {
    (
        name: $name:ident
        types: {
            $($type_name:ident = $type_path:ty),+ $(,)?
        }
        on_request: |$ctx:ident $(, $this:ident)? | $on_request_body:expr
            $(,)?
    ) => {
        impl $crate::ServerHandler for $name {
            $(
                type $type_name = $type_path;
            )+

            #[allow(clippy::type_complexity)]
            fn on_request<'a>(
                &'a self,
                $ctx: $crate::ServerCtx<Self::Request, Self::Response, Self::GlobalData, Self::LocalData>,
            ) -> std::pin::Pin<std::boxed::Box<dyn std::future::Future<
                Output = std::io::Result<Self::Response>
            > + Send + 'a>>
            where
                Self: Sync + 'a
            {
                $(let $this = self;)?
                std::boxed::Box::pin(async move {
                    $on_request_body
                })
            }
        }
    };
}
