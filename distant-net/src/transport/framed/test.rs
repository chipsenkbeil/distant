use crate::{FramedTransport, InmemoryTransport, PlainCodec};

#[cfg(test)]
impl FramedTransport<InmemoryTransport, PlainCodec> {
    /// Makes a connected pair of framed inmemory transports with plain codec for testing purposes
    pub fn make_test_pair() -> (
        FramedTransport<InmemoryTransport, PlainCodec>,
        FramedTransport<InmemoryTransport, PlainCodec>,
    ) {
        Self::pair(100)
    }
}
