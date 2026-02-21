use std::io;

use dyn_clone::DynClone;

use super::Frame;

mod chain;
mod compression;
mod encryption;
mod plain;
mod predicate;

pub use chain::*;
pub use compression::*;
pub use encryption::*;
pub use plain::*;
pub use predicate::*;

/// Represents abstraction that implements specific encoder and decoder logic to transform an
/// arbitrary collection of bytes. This can be used to encrypt and authenticate bytes sent and
/// received by transports.
pub trait Codec: DynClone {
    /// Encodes a frame's item
    fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>>;

    /// Decodes a frame's item
    fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>>;
}

/// Represents a [`Box`]ed version of [`Codec`]
pub type BoxedCodec = Box<dyn Codec + Send + Sync>;

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
