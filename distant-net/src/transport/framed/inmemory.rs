use crate::{FramedTransportRead, FramedTransportWrite};
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    any::{type_name, Any},
    io,
};
use tokio::sync::mpsc;

pub struct InmemoryFramedTransport {
    writer: InmemoryFramedTransportWriteHalf,
    reader: InmemoryFramedTransportReadHalf,
}

#[derive(Clone)]
pub struct InmemoryFramedTransportWriteHalf {
    inner: mpsc::Sender<Box<dyn Any>>,
}

pub struct InmemoryFramedTransportReadHalf {
    inner: mpsc::Receiver<Box<dyn Any + Send>>,
}

#[async_trait]
impl FramedTransportRead for InmemoryFramedTransportReadHalf {
    async fn recv<R: DeserializeOwned>(&mut self) -> io::Result<Option<R>> {
        match self.inner.recv().await {
            Some(data) => match data.downcast::<R>() {
                Ok(data) => Ok(Some(*data)),
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Could not cast value to {}", type_name::<R>()),
                )),
            },
            None => Ok(None),
        }
    }
}
