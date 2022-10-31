use crate::common::{authentication::Authenticator, Destination, FramedTransport, Map, Transport};
use async_trait::async_trait;
use std::{future::Future, io};

pub type BoxedLaunchHandler = Box<dyn LaunchHandler>;
pub type BoxedConnectHandler = Box<dyn ConnectHandler>;

/// Represents an interface to start a server at some remote `destination`.
///
/// * `destination` is the location where the server will be started.
/// * `options` is provided to include extra information needed to launch or establish the
///   connection.
/// * `authenticator` is provided to support a challenge-based authentication while launching.
///
/// Returns a [`Destination`] representing the new origin to use if a connection is desired.
#[async_trait]
pub trait LaunchHandler: Send + Sync {
    async fn launch(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<Destination>;
}

#[async_trait]
impl<F, R> LaunchHandler for F
where
    F: for<'a> Fn(&'a Destination, &'a Map, &'a mut dyn Authenticator) -> R + Send + Sync + 'static,
    R: Future<Output = io::Result<Destination>> + Send + 'static,
{
    async fn launch(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<Destination> {
        self(destination, options, authenticator).await
    }
}

/// Represents an interface to perform a connection to some remote `destination`.
///
/// * `destination` is the location of the server to connect to.
/// * `options` is provided to include extra information needed to establish the connection.
/// * `authenticator` is provided to support a challenge-based authentication while connecting.
///
/// Returns a [`FramedTransport`] representing the connection.
#[async_trait]
pub trait ConnectHandler: Send + Sync {
    async fn connect(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<FramedTransport<Box<dyn Transport + Send + Sync>>>;
}

#[async_trait]
impl<F, R> ConnectHandler for F
where
    F: for<'a> Fn(&'a Destination, &'a Map, &'a mut dyn Authenticator) -> R + Send + Sync + 'static,
    R: Future<Output = io::Result<FramedTransport<Box<dyn Transport + Send + Sync>>>>
        + Send
        + 'static,
{
    async fn connect(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<FramedTransport<Box<dyn Transport + Send + Sync>>> {
        self(destination, options, authenticator).await
    }
}
