use crate::{
    manager::data::{Destination, Extra},
    DistantClient,
};
use async_trait::async_trait;
use distant_net::AuthClient;
use std::io;

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
