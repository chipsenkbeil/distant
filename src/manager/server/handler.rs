use crate::manager::data::{Destination, Extra};
use distant_core::DistantClient;
use std::io;
use std::{future::Future, pin::Pin};

/// Represents a handler that is triggered for a connect request
pub struct ConnectHandler(
    Box<
        dyn Fn(&Destination, &Extra) -> Pin<Box<dyn Future<Output = io::Result<DistantClient>>>>
            + Send
            + Sync,
    >,
);

impl ConnectHandler {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&Destination, &Extra) -> Pin<Box<dyn Future<Output = io::Result<DistantClient>>>>,
    {
        Self(f)
    }

    /// Connect to some remote system, returning a [`DistantClient`] to communicate with the
    /// system if successful
    pub async fn do_connect(
        &self,
        destination: &Destination,
        extra: &Extra,
    ) -> io::Result<DistantClient> {
        (self.0)(destination, extra).await
    }
}
