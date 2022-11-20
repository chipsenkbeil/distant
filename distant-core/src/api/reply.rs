use crate::{api::DistantMsg, data::DistantResponseData};
use distant_net::server::Reply;
use std::{future::Future, io, pin::Pin};

/// Wrapper around a reply that can be batch or single, converting
/// a single data into the wrapped type
pub struct DistantSingleReply(Box<dyn Reply<Data = DistantMsg<DistantResponseData>>>);

impl From<Box<dyn Reply<Data = DistantMsg<DistantResponseData>>>> for DistantSingleReply {
    fn from(reply: Box<dyn Reply<Data = DistantMsg<DistantResponseData>>>) -> Self {
        Self(reply)
    }
}

impl Reply for DistantSingleReply {
    type Data = DistantResponseData;

    fn send(&self, data: Self::Data) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + '_>> {
        self.0.send(DistantMsg::Single(data))
    }

    fn blocking_send(&self, data: Self::Data) -> io::Result<()> {
        self.0.blocking_send(DistantMsg::Single(data))
    }

    fn clone_reply(&self) -> Box<dyn Reply<Data = Self::Data>> {
        Box::new(Self(self.0.clone_reply()))
    }
}
