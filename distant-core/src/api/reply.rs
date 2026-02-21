use std::io;

use crate::net::server::Reply;

use crate::protocol;

/// Wrapper around a reply that can be batch or single, converting
/// a single data into the wrapped type
pub struct DistantSingleReply(Box<dyn Reply<Data = protocol::Msg<protocol::Response>>>);

impl From<Box<dyn Reply<Data = protocol::Msg<protocol::Response>>>> for DistantSingleReply {
    fn from(reply: Box<dyn Reply<Data = protocol::Msg<protocol::Response>>>) -> Self {
        Self(reply)
    }
}

impl Reply for DistantSingleReply {
    type Data = protocol::Response;

    fn send(&self, data: Self::Data) -> io::Result<()> {
        self.0.send(protocol::Msg::Single(data))
    }

    fn clone_reply(&self) -> Box<dyn Reply<Data = Self::Data>> {
        Box::new(Self(self.0.clone_reply()))
    }
}
