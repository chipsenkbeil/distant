use super::Frame;
use std::io;

mod plain;
mod xchacha20poly1305;

pub use plain::*;
pub use xchacha20poly1305::*;

/// Represents abstraction that implements specific encoder and decoder logic to transform an
/// arbitrary collection of bytes. This can be used to encrypt and authenticate bytes sent and
/// received by transports.
pub trait Codec: Clone {
    /// Encodes a frame's item
    fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>>;

    /// Decodes a frame's item
    fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>>;
}
