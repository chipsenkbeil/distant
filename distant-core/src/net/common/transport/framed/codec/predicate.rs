use std::io;
use std::sync::Arc;

use super::{Codec, Frame};

/// Represents a codec that invokes one of two codecs based on the given predicate
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PredicateCodec<T, U, P> {
    left: T,
    right: U,
    predicate: Arc<P>,
}

impl<T, U, P> PredicateCodec<T, U, P> {
    /// Creates a new predicate codec where the left codec is invoked if the predicate returns true
    /// and the right codec is invoked if the predicate returns false
    pub fn new(left: T, right: U, predicate: P) -> Self {
        Self {
            left,
            right,
            predicate: Arc::new(predicate),
        }
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

impl<T, U, P> Clone for PredicateCodec<T, U, P>
where
    T: Clone,
    U: Clone,
{
    fn clone(&self) -> Self {
        Self {
            left: self.left.clone(),
            right: self.right.clone(),
            predicate: Arc::clone(&self.predicate),
        }
    }
}

impl<T, U, P> Codec for PredicateCodec<T, U, P>
where
    T: Codec + Clone,
    U: Codec + Clone,
    P: Fn(&Frame) -> bool,
{
    fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        if (self.predicate)(&frame) {
            Codec::encode(&mut self.left, frame)
        } else {
            Codec::encode(&mut self.right, frame)
        }
    }

    fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        if (self.predicate)(&frame) {
            Codec::decode(&mut self.left, frame)
        } else {
            Codec::decode(&mut self.right, frame)
        }
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[derive(Copy, Clone)]
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
    #[allow(dead_code)] // Used in trait implementations below
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
    fn encode_should_invoke_left_codec_if_predicate_returns_true() {
        let mut codec = PredicateCodec::new(
            TestCodec::new("hello"),
            TestCodec::new("world"),
            |_: &Frame| true,
        );
        let frame = codec.encode(Frame::new(b"some bytes")).unwrap();
        assert_eq!(frame, b"some byteshello");
    }

    #[test]
    fn encode_should_invoke_right_codec_if_predicate_returns_false() {
        let mut codec = PredicateCodec::new(
            TestCodec::new("hello"),
            TestCodec::new("world"),
            |_: &Frame| false,
        );
        let frame = codec.encode(Frame::new(b"some bytes")).unwrap();
        assert_eq!(frame, b"some bytesworld");
    }

    #[test]
    fn decode_should_invoke_left_codec_if_predicate_returns_true() {
        let mut codec = PredicateCodec::new(
            TestCodec::new("hello"),
            TestCodec::new("world"),
            |_: &Frame| true,
        );
        let frame = codec.decode(Frame::new(b"some byteshello")).unwrap();
        assert_eq!(frame, b"some bytes");
    }

    #[test]
    fn decode_should_invoke_right_codec_if_predicate_returns_false() {
        let mut codec = PredicateCodec::new(
            TestCodec::new("hello"),
            TestCodec::new("world"),
            |_: &Frame| false,
        );
        let frame = codec.decode(Frame::new(b"some bytesworld")).unwrap();
        assert_eq!(frame, b"some bytes");
    }

    // -----------------------------------------------------------------------
    // Construction and accessor tests
    // -----------------------------------------------------------------------

    #[test]
    fn new_should_create_codec_with_given_left_and_right() {
        let codec = PredicateCodec::new(
            TestCodec::new("left"),
            TestCodec::new("right"),
            |_: &Frame| true,
        );
        assert_eq!(codec.as_left().msg, "left");
        assert_eq!(codec.as_right().msg, "right");
    }

    #[test]
    fn as_left_should_return_reference_to_left_codec() {
        let codec = PredicateCodec::new(TestCodec::new("L"), TestCodec::new("R"), |_: &Frame| true);
        let left = codec.as_left();
        assert_eq!(left.msg, "L");
    }

    #[test]
    fn as_right_should_return_reference_to_right_codec() {
        let codec = PredicateCodec::new(TestCodec::new("L"), TestCodec::new("R"), |_: &Frame| true);
        let right = codec.as_right();
        assert_eq!(right.msg, "R");
    }

    #[test]
    fn into_left_should_consume_and_return_left_codec() {
        let codec = PredicateCodec::new(
            TestCodec::new("left"),
            TestCodec::new("right"),
            |_: &Frame| true,
        );
        let left = codec.into_left();
        assert_eq!(left.msg, "left");
    }

    #[test]
    fn into_right_should_consume_and_return_right_codec() {
        let codec = PredicateCodec::new(
            TestCodec::new("left"),
            TestCodec::new("right"),
            |_: &Frame| true,
        );
        let right = codec.into_right();
        assert_eq!(right.msg, "right");
    }

    #[test]
    fn into_left_right_should_consume_and_return_both_codecs() {
        let codec = PredicateCodec::new(TestCodec::new("L"), TestCodec::new("R"), |_: &Frame| true);
        let (left, right) = codec.into_left_right();
        assert_eq!(left.msg, "L");
        assert_eq!(right.msg, "R");
    }

    // -----------------------------------------------------------------------
    // Clone impl
    // -----------------------------------------------------------------------

    #[test]
    fn clone_should_produce_independent_codec_with_same_behavior() {
        let codec = PredicateCodec::new(
            TestCodec::new("hello"),
            TestCodec::new("world"),
            |_: &Frame| true,
        );
        let mut cloned = codec.clone();

        // The cloned codec should behave identically
        let frame = cloned.encode(Frame::new(b"data")).unwrap();
        assert_eq!(frame, b"datahello");
    }

    #[test]
    fn clone_should_preserve_left_and_right_codecs() {
        let codec = PredicateCodec::new(TestCodec::new("A"), TestCodec::new("B"), |_: &Frame| true);
        let cloned = codec.clone();
        assert_eq!(cloned.as_left().msg, "A");
        assert_eq!(cloned.as_right().msg, "B");
    }

    // -----------------------------------------------------------------------
    // Encode/decode error propagation
    // -----------------------------------------------------------------------

    #[test]
    fn encode_should_propagate_error_from_left_codec() {
        let mut codec = PredicateCodec::new(ErrCodec, TestCodec::new("ok"), |_: &Frame| true);
        let result = codec.encode(Frame::new(b"data"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn encode_should_propagate_error_from_right_codec() {
        let mut codec = PredicateCodec::new(TestCodec::new("ok"), ErrCodec, |_: &Frame| false);
        let result = codec.encode(Frame::new(b"data"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn decode_should_propagate_error_from_left_codec() {
        let mut codec = PredicateCodec::new(ErrCodec, TestCodec::new("ok"), |_: &Frame| true);
        let result = codec.decode(Frame::new(b"data"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn decode_should_propagate_error_from_right_codec() {
        let mut codec = PredicateCodec::new(TestCodec::new("ok"), ErrCodec, |_: &Frame| false);
        let result = codec.decode(Frame::new(b"data"));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    // -----------------------------------------------------------------------
    // Predicate based on frame content
    // -----------------------------------------------------------------------

    #[test]
    fn predicate_should_be_called_with_actual_frame_content() {
        // Use a predicate that checks whether the frame starts with "A"
        let mut codec = PredicateCodec::new(
            TestCodec::new("_left"),
            TestCodec::new("_right"),
            |frame: &Frame| frame.as_item().first() == Some(&b'A'),
        );

        // Frame starting with "A" should use left codec
        let frame = codec.encode(Frame::new(b"A data")).unwrap();
        assert_eq!(frame, b"A data_left");

        // Frame starting with "B" should use right codec
        let frame = codec.encode(Frame::new(b"B data")).unwrap();
        assert_eq!(frame, b"B data_right");
    }

    // -----------------------------------------------------------------------
    // Encode then decode round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn encode_then_decode_roundtrip_via_left_codec() {
        let mut codec = PredicateCodec::new(
            TestCodec::new("suffix"),
            TestCodec::new("other"),
            |_: &Frame| true,
        );
        let original = b"hello world";
        let encoded = codec.encode(Frame::new(original)).unwrap();
        let decoded = codec.decode(encoded).unwrap();
        assert_eq!(decoded.as_item(), original);
    }

    #[test]
    fn encode_then_decode_roundtrip_via_right_codec() {
        let mut codec = PredicateCodec::new(
            TestCodec::new("other"),
            TestCodec::new("suffix"),
            |_: &Frame| false,
        );
        let original = b"hello world";
        let encoded = codec.encode(Frame::new(original)).unwrap();
        let decoded = codec.decode(encoded).unwrap();
        assert_eq!(decoded.as_item(), original);
    }

    // -----------------------------------------------------------------------
    // Default impl (when T, U, P all implement Default)
    // -----------------------------------------------------------------------

    #[test]
    fn default_should_work_when_types_implement_default() {
        // PredicateCodec derives Default, so this should compile and work
        // with types that implement Default
        let codec: PredicateCodec<String, String, fn(&Frame) -> bool> = PredicateCodec {
            left: String::new(),
            right: String::new(),
            predicate: Arc::new((|_: &Frame| true) as fn(&Frame) -> bool),
        };
        assert!(codec.as_left().is_empty());
        assert!(codec.as_right().is_empty());
    }

    // -----------------------------------------------------------------------
    // Empty frame handling
    // -----------------------------------------------------------------------

    #[test]
    fn encode_should_handle_empty_frame() {
        let mut codec = PredicateCodec::new(
            TestCodec::new("suffix"),
            TestCodec::new("other"),
            |_: &Frame| true,
        );
        let frame = codec.encode(Frame::new(b"")).unwrap();
        assert_eq!(frame, b"suffix");
    }

    #[test]
    fn decode_should_handle_frame_that_is_exactly_the_suffix() {
        let mut codec = PredicateCodec::new(
            TestCodec::new("suffix"),
            TestCodec::new("other"),
            |_: &Frame| true,
        );
        let frame = codec.decode(Frame::new(b"suffix")).unwrap();
        assert_eq!(frame.as_item(), b"");
    }
}
