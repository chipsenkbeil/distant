use bytes::{Buf, BufMut, BytesMut};
use std::convert::TryInto;
use tokio::io;
use tokio_util::codec::{Decoder, Encoder};

/// Total size in bytes that is used for storing length
static LEN_SIZE: usize = 8;

#[inline]
fn frame_size(msg_size: usize) -> usize {
    // u64 (8 bytes) + msg size
    LEN_SIZE + msg_size
}

/// Represents the codec to encode and decode data for transmission
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DistantCodec;

impl<'a> Encoder<&'a [u8]> for DistantCodec {
    type Error = io::Error;

    fn encode(&mut self, item: &'a [u8], dst: &mut BytesMut) -> Result<(), Self::Error> {
        // Add our full frame to the bytes
        dst.reserve(frame_size(item.len()));
        dst.put_u64(item.len() as u64);
        dst.put(item);

        Ok(())
    }
}

impl Decoder for DistantCodec {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // First, check if we have more data than just our frame's message length
        if src.len() <= LEN_SIZE {
            return Ok(None);
        }

        // Second, retrieve total size of our frame's message
        let msg_len = u64::from_be_bytes(src[..LEN_SIZE].try_into().unwrap());

        // Third, return our msg if it's available, stripping it of the length data
        let frame_len = frame_size(msg_len as usize);
        if src.len() >= frame_len {
            let data = src[LEN_SIZE..frame_len].to_vec();

            // Advance so frame is no longer kept around
            src.advance(frame_len);

            Ok(Some(data))
        } else {
            Ok(None)
        }
    }
}
