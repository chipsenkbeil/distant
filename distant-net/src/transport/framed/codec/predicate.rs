use super::{Codec, Frame};
use std::{io, sync::Arc};

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
    T: Codec,
    U: Codec,
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
}
