use crate::{IntoSplit, TypedAsyncRead, TypedAsyncWrite, TypedTransport};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::io;

/// Interface representing a transport that uses [`serde`] to serialize and deserialize data
pub trait SerdeTransport:
    SerdeTransportRead + SerdeTransportWrite + IntoSplit<Write = Self::WriteHalf, Read = Self::ReadHalf>
{
    type ReadHalf: SerdeTransportRead;
    type WriteHalf: SerdeTransportWrite;
}

/// Interface representing a transport's read half that uses [`serde`] to deserialize data
#[async_trait]
pub trait SerdeTransportRead {
    /// Attempts to read some data as `T`, returning [`io::Error`] if unable to deserialize
    /// or some other error occurs. `Some(T)` is returned if successful. `None` is
    /// returned if no more data is available.
    async fn read<T>(&mut self) -> io::Result<Option<T>>
    where
        T: DeserializeOwned;
}

/// Interface representing a transport's write half that uses [`serde`] to serialize data
#[async_trait]
pub trait SerdeTransportWrite {
    /// Attempts to write some data of type `T`, returning [`io::Error`] if unable to serialize
    /// or some other error occurs.
    async fn write<T>(&mut self, data: T) -> io::Result<()>
    where
        T: Serialize + Send + 'static;
}

impl<T, W, R> TypedTransport<W, R> for T
where
    T: SerdeTransport + Send,
    W: Serialize + Send + 'static,
    R: DeserializeOwned,
    T::ReadHalf: Send,
    T::WriteHalf: Send,
{
    type ReadHalf = T::ReadHalf;
    type WriteHalf = T::WriteHalf;
}

#[async_trait]
impl<W, T> TypedAsyncWrite<T> for W
where
    W: SerdeTransportWrite + Send,
    T: Serialize + Send + 'static,
{
    async fn write(&mut self, data: T) -> io::Result<()> {
        W::write(self, data).await
    }
}

#[async_trait]
impl<R, T> TypedAsyncRead<T> for R
where
    R: SerdeTransportRead + Send,
    T: DeserializeOwned,
{
    async fn read(&mut self) -> io::Result<Option<T>> {
        R::read(self).await
    }
}
