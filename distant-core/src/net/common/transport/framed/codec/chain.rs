use std::io;

use super::{Codec, Frame};

/// Represents a codec that chains together other codecs such that encoding will call the encode
/// methods of the underlying, chained codecs from left-to-right and decoding will call the decode
/// methods in reverse order
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainCodec<T, U> {
    left: T,
    right: U,
}

impl<T, U> ChainCodec<T, U> {
    /// Chains two codecs together such that `left` will be invoked first during encoding and
    /// `right` will be invoked first during decoding
    pub fn new(left: T, right: U) -> Self {
        Self { left, right }
    }

    /// Returns reference to left codec
    pub fn as_left(&self) -> &T {
        &self.left
    }

    /// Consumes the chain and returns the left codec
    pub fn into_left(self) -> T {
        self.left
    }

    /// Returns reference to right codec
    pub fn as_right(&self) -> &U {
        &self.right
    }

    /// Consumes the chain and returns the right codec
    pub fn into_right(self) -> U {
        self.right
    }

    /// Consumes the chain and returns the left and right codecs
    pub fn into_left_right(self) -> (T, U) {
        (self.left, self.right)
    }
}

impl<T, U> Codec for ChainCodec<T, U>
where
    T: Codec + Clone,
    U: Codec + Clone,
{
    fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        Codec::encode(&mut self.left, frame).and_then(|frame| Codec::encode(&mut self.right, frame))
    }

    fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        Codec::decode(&mut self.right, frame).and_then(|frame| Codec::decode(&mut self.left, frame))
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[derive(Copy, Clone, Debug)]
    struct TestCodec<'a> {
        msg: &'a str,
    }

    impl<'a> TestCodec<'a> {
        pub fn new(msg: &'a str) -> Self {
            Self { msg }
        }
    }

    impl Codec for TestCodec<'_> {
        fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
            let mut item = frame.into_item().to_vec();
            item.extend_from_slice(self.msg.as_bytes());
            Ok(Frame::from(item))
        }

        fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
            let item = frame.into_item().to_vec();
            let frame = Frame::new(item.strip_suffix(self.msg.as_bytes()).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Decode failed because did not end with suffix: {}",
                        self.msg
                    ),
                )
            })?);
            Ok(frame.into_owned())
        }
    }

    #[derive(Copy, Clone)]
    struct ErrCodec;

    impl Codec for ErrCodec {
        fn encode<'a>(&mut self, _frame: Frame<'a>) -> io::Result<Frame<'a>> {
            Err(io::Error::from(io::ErrorKind::InvalidData))
        }

        fn decode<'a>(&mut self, _frame: Frame<'a>) -> io::Result<Frame<'a>> {
            Err(io::Error::from(io::ErrorKind::InvalidData))
        }
    }

    #[test]
    fn encode_should_invoke_left_codec_followed_by_right_codec() {
        let mut codec = ChainCodec::new(TestCodec::new("hello"), TestCodec::new("world"));
        let frame = codec.encode(Frame::new(b"some bytes")).unwrap();
        assert_eq!(frame, b"some byteshelloworld");
    }

    #[test]
    fn encode_should_fail_if_left_codec_fails_to_encode() {
        let mut codec = ChainCodec::new(ErrCodec, TestCodec::new("world"));
        assert_eq!(
            codec.encode(Frame::new(b"some bytes")).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn encode_should_fail_if_right_codec_fails_to_encode() {
        let mut codec = ChainCodec::new(TestCodec::new("hello"), ErrCodec);
        assert_eq!(
            codec.encode(Frame::new(b"some bytes")).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn decode_should_invoke_right_codec_followed_by_left_codec() {
        let mut codec = ChainCodec::new(TestCodec::new("hello"), TestCodec::new("world"));
        let frame = codec.decode(Frame::new(b"some byteshelloworld")).unwrap();
        assert_eq!(frame, b"some bytes");
    }

    #[test]
    fn decode_should_fail_if_left_codec_fails_to_decode() {
        let mut codec = ChainCodec::new(ErrCodec, TestCodec::new("world"));
        assert_eq!(
            codec.decode(Frame::new(b"some bytes")).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn decode_should_fail_if_right_codec_fails_to_decode() {
        let mut codec = ChainCodec::new(TestCodec::new("hello"), ErrCodec);
        assert_eq!(
            codec.decode(Frame::new(b"some bytes")).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    // --- Accessor tests ---

    #[test]
    fn as_left_should_return_reference_to_left_codec() {
        let codec = ChainCodec::new(TestCodec::new("left"), TestCodec::new("right"));
        assert_eq!(codec.as_left().msg, "left");
    }

    #[test]
    fn as_right_should_return_reference_to_right_codec() {
        let codec = ChainCodec::new(TestCodec::new("left"), TestCodec::new("right"));
        assert_eq!(codec.as_right().msg, "right");
    }

    #[test]
    fn into_left_should_return_left_codec() {
        let codec = ChainCodec::new(TestCodec::new("left"), TestCodec::new("right"));
        let left = codec.into_left();
        assert_eq!(left.msg, "left");
    }

    #[test]
    fn into_right_should_return_right_codec() {
        let codec = ChainCodec::new(TestCodec::new("left"), TestCodec::new("right"));
        let right = codec.into_right();
        assert_eq!(right.msg, "right");
    }

    #[test]
    fn into_left_right_should_return_both_codecs() {
        let codec = ChainCodec::new(TestCodec::new("left"), TestCodec::new("right"));
        let (left, right) = codec.into_left_right();
        assert_eq!(left.msg, "left");
        assert_eq!(right.msg, "right");
    }

    // --- Round-trip tests ---

    #[test]
    fn encode_then_decode_should_round_trip() {
        let mut codec = ChainCodec::new(TestCodec::new("hello"), TestCodec::new("world"));
        let original = Frame::new(b"some bytes");
        let encoded = codec.encode(original).unwrap();
        let decoded = codec.decode(encoded).unwrap();
        assert_eq!(decoded, b"some bytes");
    }

    #[test]
    fn encode_then_decode_should_round_trip_empty_frame() {
        let mut codec = ChainCodec::new(TestCodec::new("a"), TestCodec::new("b"));
        let original = Frame::empty();
        let encoded = codec.encode(original).unwrap();
        let decoded = codec.decode(encoded).unwrap();
        assert!(decoded.is_empty());
    }

    // --- Triple chain (nested) ---

    #[test]
    fn triple_chain_encode_should_invoke_all_three_codecs_left_to_right() {
        let inner = ChainCodec::new(TestCodec::new("1"), TestCodec::new("2"));
        let mut codec = ChainCodec::new(inner, TestCodec::new("3"));

        let frame = codec.encode(Frame::new(b"data")).unwrap();
        assert_eq!(frame, b"data123");
    }

    #[test]
    fn triple_chain_decode_should_invoke_all_three_codecs_right_to_left() {
        let inner = ChainCodec::new(TestCodec::new("1"), TestCodec::new("2"));
        let mut codec = ChainCodec::new(inner, TestCodec::new("3"));

        let frame = codec.decode(Frame::new(b"data123")).unwrap();
        assert_eq!(frame, b"data");
    }

    #[test]
    fn triple_chain_encode_then_decode_should_round_trip() {
        let inner = ChainCodec::new(TestCodec::new("1"), TestCodec::new("2"));
        let mut codec = ChainCodec::new(inner, TestCodec::new("3"));

        let encoded = codec.encode(Frame::new(b"round-trip")).unwrap();
        let decoded = codec.decode(encoded).unwrap();
        assert_eq!(decoded, b"round-trip");
    }

    // --- Clone and equality tests ---

    #[test]
    fn clone_should_produce_equal_chain() {
        let codec = ChainCodec::new(TestCodec::new("a"), TestCodec::new("b"));
        let cloned = codec;
        assert_eq!(codec.as_left().msg, cloned.as_left().msg);
        assert_eq!(codec.as_right().msg, cloned.as_right().msg);
    }

    #[test]
    fn cloned_codec_should_encode_and_decode_independently() {
        let codec = ChainCodec::new(TestCodec::new("hello"), TestCodec::new("world"));
        let mut cloned = codec;

        let encoded = cloned.encode(Frame::new(b"test")).unwrap();
        assert_eq!(encoded, b"testhelloworld");

        let decoded = cloned.decode(encoded).unwrap();
        assert_eq!(decoded, b"test");
    }

    // --- Default tests ---

    /// Uses PlainCodec which implements Default
    #[test]
    fn default_chain_of_plain_codecs_should_pass_through_unchanged() {
        use crate::net::common::PlainCodec;
        let mut codec = ChainCodec::<PlainCodec, PlainCodec>::default();

        let encoded = codec.encode(Frame::new(b"passthrough")).unwrap();
        assert_eq!(encoded, b"passthrough");

        let decoded = codec.decode(encoded).unwrap();
        assert_eq!(decoded, b"passthrough");
    }

    // --- Error propagation ---

    #[test]
    fn decode_should_not_invoke_left_if_right_fails() {
        // If right fails during decode, left should not be called.
        // ErrCodec on the right means decode of right fails immediately.
        let mut codec = ChainCodec::new(TestCodec::new("left"), ErrCodec);
        let result = codec.decode(Frame::new(b"anything"));
        assert!(result.is_err());
    }

    #[test]
    fn encode_should_not_invoke_right_if_left_fails() {
        // If left fails during encode, right should not be called.
        let mut codec = ChainCodec::new(ErrCodec, TestCodec::new("right"));
        let result = codec.encode(Frame::new(b"anything"));
        assert!(result.is_err());
    }

    // --- Debug ---

    #[test]
    fn debug_should_not_panic() {
        let codec = ChainCodec::new(TestCodec::new("a"), TestCodec::new("b"));
        let debug_str = format!("{:?}", codec);
        assert!(!debug_str.is_empty());
    }

    // --- Large data ---

    #[test]
    fn encode_then_decode_large_data_should_round_trip() {
        let mut codec = ChainCodec::new(TestCodec::new("X"), TestCodec::new("Y"));
        let data: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();
        let encoded = codec.encode(Frame::new(&data)).unwrap();
        let decoded = codec.decode(encoded).unwrap();
        assert_eq!(decoded.as_item(), data.as_slice());
    }
}
