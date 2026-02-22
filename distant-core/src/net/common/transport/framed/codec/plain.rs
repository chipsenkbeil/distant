use std::io;

use super::{Codec, Frame};

/// Represents a codec that does not alter the frame (synonymous with "plain text")
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct PlainCodec;

impl PlainCodec {
    pub fn new() -> Self {
        Self
    }
}

impl Codec for PlainCodec {
    fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        Ok(frame)
    }

    fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        Ok(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    #[test]
    fn new_creates_instance() {
        let codec = PlainCodec::new();
        // Verify it is a unit struct (no data to check, just that it compiles and runs)
        assert_eq!(codec, PlainCodec);
    }

    #[test]
    fn default_creates_instance() {
        let codec = PlainCodec;
        assert_eq!(codec, PlainCodec::new());
    }

    #[test]
    fn encode_returns_same_frame_for_nonempty_data() {
        let mut codec = PlainCodec::new();
        let data = b"hello, world";
        let frame = Frame::new(data);
        let encoded = codec.encode(frame).expect("encode failed");
        assert_eq!(encoded.as_item(), data);
    }

    #[test]
    fn encode_returns_same_frame_for_empty_data() {
        let mut codec = PlainCodec::new();
        let frame = Frame::empty();
        let encoded = codec.encode(frame).expect("encode failed");
        assert!(encoded.is_empty());
    }

    #[test]
    fn decode_returns_same_frame_for_nonempty_data() {
        let mut codec = PlainCodec::new();
        let data = b"some binary data \x00\xff";
        let frame = Frame::new(data);
        let decoded = codec.decode(frame).expect("decode failed");
        assert_eq!(decoded.as_item(), data);
    }

    #[test]
    fn decode_returns_same_frame_for_empty_data() {
        let mut codec = PlainCodec::new();
        let frame = Frame::empty();
        let decoded = codec.decode(frame).expect("decode failed");
        assert!(decoded.is_empty());
    }

    #[test]
    fn encode_then_decode_round_trip() {
        let mut codec = PlainCodec::new();
        let data = b"round trip payload";
        let original = Frame::new(data);
        let encoded = codec.encode(original).expect("encode failed");
        let decoded = codec.decode(encoded).expect("decode failed");
        assert_eq!(decoded.as_item(), data);
    }

    #[test]
    fn partial_eq_works() {
        let a = PlainCodec::new();
        let b = PlainCodec;
        assert_eq!(a, b);
    }

    #[test]
    fn clone_produces_equal_instance() {
        let original = PlainCodec::new();
        let cloned = original;
        assert_eq!(original, cloned);
    }

    #[test]
    fn encode_preserves_large_frame() {
        let mut codec = PlainCodec::new();
        let data: Vec<u8> = (0..=255).cycle().take(4096).collect();
        let frame = Frame::new(&data);
        let encoded = codec.encode(frame).expect("encode failed");
        assert_eq!(encoded.as_item(), data.as_slice());
    }
}
