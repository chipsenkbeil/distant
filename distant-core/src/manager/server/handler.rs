use crate::{
    manager::data::{Destination, Extra},
    DistantMsg, DistantRequestData, DistantResponseData,
};
use distant_net::{AuthClient, Request, Response, TypedAsyncRead, TypedAsyncWrite};
use std::{future::Future, io, pin::Pin};

pub type BoxedDistantWriter =
    Box<dyn TypedAsyncWrite<Request<DistantMsg<DistantRequestData>>> + Send>;
pub type BoxedDistantReader =
    Box<dyn TypedAsyncRead<Response<DistantMsg<DistantResponseData>>> + Send>;
pub type BoxedDistantWriterReader = (BoxedDistantWriter, BoxedDistantReader);
pub type BoxedConnectHandler = Box<dyn ConnectHandler>;

/// Used to connect to a destination, returning a connected reader and writer pair
pub trait ConnectHandler: Send + Sync {
    fn connect(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &AuthClient,
    ) -> Pin<Box<dyn Future<Output = io::Result<BoxedDistantWriterReader>> + Send>>;
}

impl<F, R> ConnectHandler for F
where
    F: for<'a> Fn(&'a Destination, &'a Extra, &'a AuthClient) -> R + Send + Sync + 'static,
    R: Future<Output = io::Result<BoxedDistantWriterReader>> + Send + 'static,
{
    fn connect(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &AuthClient,
    ) -> Pin<Box<dyn Future<Output = io::Result<BoxedDistantWriterReader>> + Send>> {
        Box::pin(self(destination, extra, auth_client))
    }
}
