use crate::{
    manager::data::{Destination, Extra},
    DistantClient,
};
use async_trait::async_trait;
use distant_net::AuthClient;
use std::{future::Future, io};

/// Interface used to connect to the specified destination, returning a connected client
#[async_trait]
pub trait ConnectHandler {
    /// Attempt to connect to the specified destination, returning a connected client if successful
    async fn connect(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth: &AuthClient,
    ) -> io::Result<DistantClient>;
}

#[async_trait]
impl<F, R> ConnectHandler for F
where
    F: Fn(&Destination, &Extra, &AuthClient) -> R + Send + Sync,
    R: Future<Output = io::Result<DistantClient>> + Send + Sync,
{
    async fn connect(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth: &AuthClient,
    ) -> io::Result<DistantClient> {
        self(destination, extra, auth).await
    }
}
