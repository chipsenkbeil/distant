use super::{Codec, Frame};
use std::io;

/// Represents a codec that chains together other codecs such that encoding will call the encode
/// methods of the underlying, chained codecs from left-to-right and decoding will call the decode
/// methods in reverse order
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ChainCodec<T, U> {
    left: T,
    right: U,
}

impl<T, U> ChainCodec<T, U> {
    /// Chains two codecs together such that `codec_1` will be invoked first during encoding and
    /// `codec_2` will be invoked first during decoding
    pub fn new(codec_1: T, codec_2: U) -> Self {
        Self {
            left: codec_1,
            right: codec_2,
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

impl<T, U> Codec for ChainCodec<T, U>
where
    T: Codec,
    U: Codec,
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
    fn encode_should_invoke_first_codec_followed_by_second_codec() {
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
    fn decode_should_invoke_second_codec_followed_by_first_codec() {
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
}
