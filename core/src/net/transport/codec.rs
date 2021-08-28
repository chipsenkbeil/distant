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
        if msg_len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Frame cannot have msg len of 0",
            ));
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoder_should_encode_byte_slice_with_frame_size() {
        let mut encoder = DistantCodec;
        let mut buf = BytesMut::new();

        // Verify that first encoding properly includes size and data
        // Format is {N as 8 bytes}{data as N bytes}
        encoder.encode(&[1, 2, 3], &mut buf).unwrap();
        assert_eq!(
            buf,
            vec![/* Size of 3 as u64 */ 0, 0, 0, 0, 0, 0, 0, 3, /* Data */ 1, 2, 3],
        );

        // Verify that second encoding properly adds to end of buffer and doesn't overwrite
        encoder.encode(&[4, 5, 6, 7, 8, 9], &mut buf).unwrap();
        assert_eq!(
            buf,
            vec![
                /* First encoding */ 0, 0, 0, 0, 0, 0, 0, 3, 1, 2, 3,
                /* Second encoding */ 0, 0, 0, 0, 0, 0, 0, 6, 4, 5, 6, 7, 8, 9,
            ],
        );
    }

    #[test]
    fn decoder_should_return_none_if_received_data_smaller_than_frame_length_field() {
        let mut decoder = DistantCodec;
        let mut buf = BytesMut::new();

        // Put 1 less than frame len field size
        for i in 0..LEN_SIZE {
            buf.put_u8(i as u8);
        }

        match decoder.decode(&mut buf) {
            Ok(None) => {}
            x => panic!("decoder.decode(...) wanted Ok(None), but got {:?}", x),
        }
    }

    #[test]
    fn decoder_should_return_none_if_received_data_is_not_a_full_frame() {
        let mut decoder = DistantCodec;
        let mut buf = BytesMut::new();

        // Put the length of our frame, but no frame at all
        buf.put_u64(4);

        match decoder.decode(&mut buf) {
            Ok(None) => {}
            x => panic!("decoder.decode(...) wanted Ok(None), but got {:?}", x),
        }

        // Put part of the frame, but not the full frame (3 out of 4 bytes)
        buf.put_u8(1);
        buf.put_u8(2);
        buf.put_u8(3);

        match decoder.decode(&mut buf) {
            Ok(None) => {}
            x => panic!("decoder.decode(...) wanted Ok(None), but got {:?}", x),
        }
    }

    #[test]
    fn decoder_should_decode_and_return_next_frame_if_available() {
        let mut decoder = DistantCodec;
        let mut buf = BytesMut::new();

        // Put exactly a frame via the length and then the data
        buf.put_u64(4);
        buf.put_u8(1);
        buf.put_u8(2);
        buf.put_u8(3);
        buf.put_u8(4);

        match decoder.decode(&mut buf) {
            Ok(Some(data)) => assert_eq!(data, [1, 2, 3, 4]),
            x => panic!(
                "decoder.decode(...) wanted Ok(Vec[1, 2, 3, 4]), but got {:?}",
                x
            ),
        }
    }

    #[test]
    fn decoder_should_properly_remove_decoded_frame_from_byte_buffer() {
        let mut decoder = DistantCodec;
        let mut buf = BytesMut::new();

        // Put exactly a frame via the length and then the data
        buf.put_u64(4);
        buf.put_u8(1);
        buf.put_u8(2);
        buf.put_u8(3);
        buf.put_u8(4);

        // Add a little bit more post frame
        buf.put_u8(123);

        match decoder.decode(&mut buf) {
            Ok(Some(data)) => {
                assert_eq!(data, [1, 2, 3, 4]);
                assert_eq!(buf, vec![123]);
            }
            x => panic!(
                "decoder.decode(...) wanted Ok(Vec[1, 2, 3, 4]), but got {:?}",
                x
            ),
        }
    }

    #[test]
    fn decoder_should_return_error_if_frame_has_msg_len_of_zero() {
        let mut decoder = DistantCodec;
        let mut buf = BytesMut::new();

        // Put a bad frame with a msg len of 0
        buf.put_u64(0);
        buf.put_u8(1);

        match decoder.decode(&mut buf) {
            Err(x) => assert_eq!(x.kind(), io::ErrorKind::InvalidData),
            x => panic!(
                "decoder.decode(...) wanted Err(io::ErrorKind::InvalidData), but got {:?}",
                x
            ),
        }
    }
}
