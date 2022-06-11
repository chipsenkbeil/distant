use crate::{FramedTransport, InmemoryTransport, PlainCodec};

#[cfg(test)]
impl FramedTransport<InmemoryTransport, PlainCodec> {
    /// Makes a connected pair of inmemory transports
    pub fn make_pair() -> (
        FramedTransport<InmemoryTransport, PlainCodec>,
        FramedTransport<InmemoryTransport, PlainCodec>,
    ) {
        Self::pair(100)
    }
}
