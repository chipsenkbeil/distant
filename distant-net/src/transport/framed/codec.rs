use bytes::BytesMut;
use std::io;

mod plain;
pub use plain::PlainCodec;

mod xchacha20poly1305;
pub use xchacha20poly1305::XChaCha20Poly1305Codec;

/// Represents abstraction that implements specific encoder and decoder logic to transform an
/// arbitrary collection of bytes. This can be used to encrypt and authenticate bytes sent and
/// received by transports.
pub trait Codec: Clone {
    /// Encodes some `item` as a frame, placing the result at the end of `dst`
    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> io::Result<()>;

    /// Attempts to decode a frame from `src`, returning `Some(Frame)` if a frame was found
    /// or `None` if the current `src` does not contain a frame
    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<u8>>>;
}
