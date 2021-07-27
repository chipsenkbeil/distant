use bytes::{Buf, BufMut, Bytes, BytesMut};
use derive_more::{Display, Error, From};
use tokio::io;
use tokio_util::codec::{Decoder, Encoder};

/// Represents a marker to indicate the beginning of the next message
static MSG_START: &'static [u8] = b";start;";

/// Represents a marker to indicate the end of the next message
static MSG_END: &'static [u8] = b";end;";

#[inline]
fn packet_size(msg_size: usize) -> usize {
    MSG_START.len() + msg_size + MSG_END.len()
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
        // Add our full packet to the bytes
        dst.reserve(packet_size(item.len()));
        dst.put(MSG_START);
        dst.put(item);
        dst.put(MSG_END);

        Ok(())
    }
}

impl Decoder for DistantCodec {
    type Item = Vec<u8>;
    type Error = DistantCodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // First, check if we have more data than just our markers, if not we say that it's okay
        // but that we're waiting
        if src.len() <= (MSG_START.len() + MSG_END.len()) {
            return Ok(None);
        }

        // Second, verify that our first N bytes match our start marker
        let marker_start = &src[..MSG_START.len()];
        if marker_start != MSG_START {
            return Err(DistantCodecError::CorruptMarker(Bytes::copy_from_slice(
                marker_start,
            )));
        }

        // Third, find end of message marker by scanning the available bytes, and
        // consume a full packet of bytes
        let mut maybe_frame = None;
        for i in (MSG_START.len() + 1)..(src.len() - MSG_END.len()) {
            let marker_end = &src[i..(i + MSG_END.len())];
            if marker_end == MSG_END {
                maybe_frame = Some(src.split_to(i + MSG_END.len()));
                break;
            }
        }

        // Fourth, return our msg if it's available, stripping it of the start and end markers
        if let Some(frame) = maybe_frame {
            let data = &frame[MSG_START.len()..(frame.len() - MSG_END.len())];

            // Advance so frame is no longer kept around
            src.advance(frame.len());

            Ok(Some(data.to_vec()))
        } else {
            Ok(None)
        }
    }
}
