use crate::Codec;
use bytes::{Buf, BufMut, BytesMut};
use std::{convert::TryInto, io};

/// Total bytes to use as the len field denoting a frame's size
const LEN_SIZE: usize = 8;

/// Represents a codec that just ships messages back and forth with no encryption or authentication
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PlainCodec;

impl PlainCodec {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Codec for PlainCodec {
    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> io::Result<()> {
        // Validate that we can fit the message plus nonce +
        if item.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Empty item provided",
            ));
        }

        dst.reserve(8 + item.len());

        // Add data in form of {LEN}{ITEM}
        dst.put_u64((item.len()) as u64);
        dst.put_slice(item);

        Ok(())
    }

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<u8>>> {
        // First, check if we have more data than just our frame's message length
        if src.len() <= LEN_SIZE {
            return Ok(None);
        }

        // Second, retrieve total size of our frame's message
        let msg_len = u64::from_be_bytes(src[..LEN_SIZE].try_into().unwrap()) as usize;
        if msg_len == 0 {
            // Ensure we advance to remove the frame
            src.advance(LEN_SIZE);

            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Frame's msg cannot have length of 0",
            ));
        }

        // Third, check if we have all data for our frame; if not, exit early
        if src.len() < msg_len + LEN_SIZE {
            return Ok(None);
        }

        // Fourth, get and return our item
        let item = src[LEN_SIZE..(LEN_SIZE + msg_len)].to_vec();

        // Fifth, advance so frame is no longer kept around
        src.advance(LEN_SIZE + msg_len);

        Ok(Some(item))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_should_fail_when_item_is_zero_bytes() {
        let mut codec = PlainCodec::new();

        let mut buf = BytesMut::new();
        let result = codec.encode(&[], &mut buf);

        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidInput => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn encode_should_build_a_frame_containing_a_length_and_item() {
        let mut codec = PlainCodec::new();

        let mut buf = BytesMut::new();
        codec
            .encode(b"hello, world", &mut buf)
            .expect("Failed to encode");

        let len = buf.get_u64() as usize;
        assert_eq!(len, 12, "Wrong length encoded");
        assert_eq!(buf.as_ref(), b"hello, world");
    }

    #[test]
    fn decode_should_return_none_if_data_smaller_than_or_equal_to_item_length_field() {
        let mut codec = PlainCodec::new();

        let mut buf = BytesMut::new();
        buf.put_bytes(0, LEN_SIZE);

        let result = codec.decode(&mut buf);
        assert!(
            matches!(result, Ok(None)),
            "Unexpected result: {:?}",
            result
        );
    }

    #[test]
    fn decode_should_return_none_if_not_enough_data_for_frame() {
        let mut codec = PlainCodec::new();

        let mut buf = BytesMut::new();
        buf.put_u64(0);

        let result = codec.decode(&mut buf);
        assert!(
            matches!(result, Ok(None)),
            "Unexpected result: {:?}",
            result
        );
    }

    #[test]
    fn decode_should_fail_if_encoded_item_length_is_zero() {
        let mut codec = PlainCodec::new();

        let mut buf = BytesMut::new();
        buf.put_u64(0);
        buf.put_u8(255);

        let result = codec.decode(&mut buf);
        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn decode_should_advance_src_by_frame_size_even_if_item_length_is_zero() {
        let mut codec = PlainCodec::new();

        let mut buf = BytesMut::new();
        buf.put_u64(0);
        buf.put_bytes(0, 3);

        assert!(
            codec.decode(&mut buf).is_err(),
            "Decode unexpectedly succeeded"
        );
        assert_eq!(buf.len(), 3, "Advanced an unexpected amount in src buf");
    }

    #[test]
    fn decode_should_advance_src_by_frame_size_when_successful() {
        let mut codec = PlainCodec::new();

        // Add 3 extra bytes after a full frame
        let mut buf = BytesMut::new();
        codec
            .encode(b"hello, world", &mut buf)
            .expect("Failed to encode");
        buf.put_bytes(0, 3);

        assert!(codec.decode(&mut buf).is_ok(), "Decode unexpectedly failed");
        assert_eq!(buf.len(), 3, "Advanced an unexpected amount in src buf");
    }

    #[test]
    fn decode_should_return_some_byte_vec_when_successful() {
        let mut codec = PlainCodec::new();

        let mut buf = BytesMut::new();
        codec
            .encode(b"hello, world", &mut buf)
            .expect("Failed to encode");

        let item = codec
            .decode(&mut buf)
            .expect("Failed to decode")
            .expect("Item not properly captured");
        assert_eq!(item, b"hello, world");
    }
}
