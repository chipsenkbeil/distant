use bytes::{Buf, BufMut, Bytes, BytesMut};
use derive_more::{Display, Error, From};
use std::convert::TryInto;
use tokio::io;
use tokio_util::codec::{Decoder, Encoder};

/// Represents a marker to indicate the beginning of the next message
static MSG_MARKER: &'static [u8] = b";msg;";

/// Total size in bytes that is used for storing length
static LEN_SIZE: usize = 8;

#[inline]
fn frame_size(msg_size: usize) -> usize {
    // MARKER + u64 (8 bytes) + msg size
    MSG_MARKER.len() + LEN_SIZE + msg_size
}

/// Possible errors that can occur during encoding and decoding
#[derive(Debug, Display, Error, From)]
pub enum DistantCodecError {
    #[display(fmt = "Corrupt Marker: {:?}", _0)]
    CorruptMarker(#[error(not(source))] Bytes),
    IoError(io::Error),
}

/// Represents the codec to encode and decode data for transmission
pub struct DistantCodec;

impl<'a> Encoder<&'a [u8]> for DistantCodec {
    type Error = DistantCodecError;

    fn encode(&mut self, item: &'a [u8], dst: &mut BytesMut) -> Result<(), Self::Error> {
        // Add our full frame to the bytes
        dst.reserve(frame_size(item.len()));
        dst.put(MSG_MARKER);
        dst.put_u64(item.len() as u64);
        dst.put(item);

        Ok(())
    }
}

impl Decoder for DistantCodec {
    type Item = Vec<u8>;
    type Error = DistantCodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // First, check if we have more data than just our markers, if not we say that it's okay
        // but that we're waiting
        if src.len() <= (MSG_MARKER.len() + LEN_SIZE) {
            return Ok(None);
        }

        // Second, verify that our first N bytes match our start marker
        let marker_start = &src[..MSG_MARKER.len()];
        if marker_start != MSG_MARKER {
            return Err(DistantCodecError::CorruptMarker(Bytes::copy_from_slice(
                marker_start,
            )));
        }

        // Third, retrieve total size of our msg
        let msg_len = u64::from_be_bytes(
            src[MSG_MARKER.len()..MSG_MARKER.len() + LEN_SIZE]
                .try_into()
                .unwrap(),
        );

        // Fourth, return our msg if it's available, stripping it of the start and end markers
        let frame_len = frame_size(msg_len as usize);
        if src.len() >= frame_len {
            let data = src[MSG_MARKER.len() + LEN_SIZE..frame_len].to_vec();

            // Advance so frame is no longer kept around
            src.advance(frame_len);

            Ok(Some(data))
        } else {
            Ok(None)
        }
    }
}
