use crate::{
    manager::data::{Destination, Extra},
    DistantMsg, DistantRequestData, DistantResponseData,
};
use async_trait::async_trait;
use distant_net::{AuthClient, Request, Response, TypedAsyncRead, TypedAsyncWrite};
use std::{future::Future, io};

pub type BoxedDistantWriter =
    Box<dyn TypedAsyncWrite<Request<DistantMsg<DistantRequestData>>> + Send>;
pub type BoxedDistantReader =
    Box<dyn TypedAsyncRead<Response<DistantMsg<DistantResponseData>>> + Send>;
pub type BoxedDistantWriterReader = (BoxedDistantWriter, BoxedDistantReader);
pub type BoxedLaunchHandler = Box<dyn LaunchHandler>;
pub type BoxedConnectHandler = Box<dyn ConnectHandler>;

/// Used to launch a server at the specified destination, returning some result as a vec of bytes
#[async_trait]
pub trait LaunchHandler: Send + Sync {
    async fn launch(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &mut AuthClient,
    ) -> io::Result<Destination>;
}

#[async_trait]
impl<F, R> LaunchHandler for F
where
    F: for<'a> Fn(&'a Destination, &'a Extra, &'a mut AuthClient) -> R + Send + Sync + 'static,
    R: Future<Output = io::Result<Destination>> + Send + 'static,
{
    async fn launch(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &mut AuthClient,
    ) -> io::Result<Destination> {
        self(destination, extra, auth_client).await
    }
}

/// Used to connect to a destination, returning a connected reader and writer pair
#[async_trait]
pub trait ConnectHandler: Send + Sync {
    async fn connect(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &mut AuthClient,
    ) -> io::Result<BoxedDistantWriterReader>;
}

#[async_trait]
impl<F, R> ConnectHandler for F
where
    F: for<'a> Fn(&'a Destination, &'a Extra, &'a mut AuthClient) -> R + Send + Sync + 'static,
    R: Future<Output = io::Result<BoxedDistantWriterReader>> + Send + 'static,
{
    async fn connect(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &mut AuthClient,
    ) -> io::Result<BoxedDistantWriterReader> {
        self(destination, extra, auth_client).await
    }
}
