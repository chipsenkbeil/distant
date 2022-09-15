use super::Frame;
use dyn_clone::DynClone;
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

/// Represents a [`Box`]ed version of [`Codec`]
pub type BoxedCodec = Box<dyn Codec + Send + Sync>;

/// Represents abstraction that implements specific encoder and decoder logic to transform an
/// arbitrary collection of bytes. This can be used to encrypt and authenticate bytes sent and
/// received by transports.
pub trait Codec: DynClone {
    /// Encodes a frame's item
    fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>>;

    /// Decodes a frame's item
    fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>>;
}

macro_rules! impl_traits {
    ($($x:tt)+) => {
        impl Clone for Box<dyn $($x)+> {
            fn clone(&self) -> Self {
                dyn_clone::clone_box(&**self)
            }
        }

        impl Codec for Box<dyn $($x)+> {
            fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
                Codec::encode(self.as_mut(), frame)
            }

            fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
                Codec::decode(self.as_mut(), frame)
            }
        }
    };
}

impl_traits!(Codec);
impl_traits!(Codec + Send);
impl_traits!(Codec + Sync);
impl_traits!(Codec + Send + Sync);

/// Interface that provides extensions to the codec interface
pub trait CodecExt {
    /// Chains this codec with another codec
    fn chain<T>(self, codec: T) -> ChainCodec<Self, T>
    where
        Self: Sized;
}

impl<C: Codec> CodecExt for C {
    fn chain<T>(self, codec: T) -> ChainCodec<Self, T> {
        ChainCodec::new(self, codec)
    }
}
