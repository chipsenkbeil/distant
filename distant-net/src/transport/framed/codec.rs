use super::Frame;
use std::io;

mod chain;
mod compression;
mod plain;
mod predicate;
mod xchacha20poly1305;

pub use chain::*;
pub use compression::*;
pub use plain::*;
pub use predicate::*;
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

/// Interface that provides extensions to the codec interface
pub trait CodecExt {
    /// Chains this codec with another codec
    fn chain<T>(self, codec: T) -> ChainCodec<Self, T>
    where
        Self: Sized;
}

impl<C: Codec> CodecExt for C {
    fn chain<T>(self, codec: T) -> ChainCodec<Self, T>
    where
        Self: Sized,
    {
        ChainCodec::new(self, codec)
    }
}
