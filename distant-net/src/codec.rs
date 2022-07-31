use bytes::BytesMut;
use std::io;
use tokio_util::codec::{Decoder, Encoder};

/// Represents abstraction of a codec that implements specific encoder and decoder for distant
pub trait Codec:
    for<'a> Encoder<&'a [u8], Error = io::Error> + Decoder<Item = Vec<u8>, Error = io::Error> + Clone
{
    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> io::Result<()>;
    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<u8>>>;
}

macro_rules! impl_traits_for_codec {
    ($type:ident) => {
        impl<'a> tokio_util::codec::Encoder<&'a [u8]> for $type {
            type Error = io::Error;

            fn encode(&mut self, item: &'a [u8], dst: &mut BytesMut) -> Result<(), Self::Error> {
                Codec::encode(self, item, dst)
            }
        }

        impl tokio_util::codec::Decoder for $type {
            type Item = Vec<u8>;
            type Error = io::Error;

            fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
                Codec::decode(self, src)
            }
        }
    };
}

mod plain;
pub use plain::PlainCodec;

mod xchacha20poly1305;
pub use xchacha20poly1305::XChaCha20Poly1305Codec;
