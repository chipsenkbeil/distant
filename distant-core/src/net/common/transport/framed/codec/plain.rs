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
