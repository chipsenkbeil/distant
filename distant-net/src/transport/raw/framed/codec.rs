use bytes::BytesMut;
use std::io;

mod plain;
pub use plain::PlainCodec;

mod xchacha20poly1305;
pub use xchacha20poly1305::XChaCha20Poly1305Codec;

/// Represents abstraction of a codec that implements specific encoder and decoder for distant
pub trait Codec: Clone {
    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> io::Result<()>;
    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<u8>>>;
}
