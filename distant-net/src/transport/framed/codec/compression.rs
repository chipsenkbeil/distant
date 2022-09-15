use super::{Codec, Frame};
use flate2::{
    bufread::{DeflateDecoder, DeflateEncoder, GzDecoder, GzEncoder, ZlibDecoder, ZlibEncoder},
    Compression,
};
use std::io::{self, Read};

/// Represents the level of compression to apply to data
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompressionLevel {
    /// Use no compression (can potentially inflate data)
    Zero = 0,

    /// Optimize for the speed of encoding
    One = 1,

    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
    Seven = 7,
    Eight = 8,

    /// Optimize for the size of data being encoded
    Nine = 9,
}

impl CompressionLevel {
    /// Applies no compression
    pub const NONE: Self = Self::Zero;

    /// Applies fastest compression
    pub const FAST: Self = Self::One;

    /// Applies best compression to reduce size (slowest)
    pub const BEST: Self = Self::Nine;
}

/// Represents a codec that applies compression during encoding and decompression during decoding
/// of a frame's item
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CompressionCodec {
    /// Apply DEFLATE compression/decompression using compression `level`
    Deflate { level: CompressionLevel },

    /// Apply gzip compression/decompression using compression `level`
    Gzip { level: CompressionLevel },

    /// Apply zlib compression/decompression using compression `level`
    Zlib { level: CompressionLevel },
}

impl CompressionCodec {
    /// Create a new deflate compression codec with the specified `level`
    pub fn deflate(level: impl Into<CompressionLevel>) -> Self {
        Self::Deflate {
            level: level.into(),
        }
    }

    /// Create a new gzip compression codec with the specified `level`
    pub fn gzip(level: impl Into<CompressionLevel>) -> Self {
        Self::Gzip {
            level: level.into(),
        }
    }

    /// Create a new zlib compression codec with the specified `level`
    pub fn zlib(level: impl Into<CompressionLevel>) -> Self {
        Self::Zlib {
            level: level.into(),
        }
    }

    /// Returns the compression level associated with the codec
    pub fn level(&self) -> CompressionLevel {
        match self {
            Self::Deflate { level } => *level,
            Self::Gzip { level } => *level,
            Self::Zlib { level } => *level,
        }
    }
}

impl Codec for CompressionCodec {
    fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        let item = frame.as_item();

        let mut buf = Vec::new();
        match *self {
            Self::Deflate { level } => {
                DeflateEncoder::new(item, Compression::new(level as u32)).read_to_end(&mut buf)?
            }
            Self::Gzip { level } => {
                GzEncoder::new(item, Compression::new(level as u32)).read_to_end(&mut buf)?
            }
            Self::Zlib { level } => {
                ZlibEncoder::new(item, Compression::new(level as u32)).read_to_end(&mut buf)?
            }
        };

        Ok(Frame::from(buf))
    }

    fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        let item = frame.as_item();

        let mut buf = Vec::new();
        match *self {
            Self::Deflate { .. } => DeflateDecoder::new(item).read_to_end(&mut buf)?,
            Self::Gzip { .. } => GzDecoder::new(item).read_to_end(&mut buf)?,
            Self::Zlib { .. } => ZlibDecoder::new(item).read_to_end(&mut buf)?,
        };

        Ok(Frame::from(buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_should_apply_appropriate_compression_algorithm() {
        // Encode using DEFLATE and verify that the compression was as expected by decompressing
        let mut codec = CompressionCodec::deflate(CompressionLevel::BEST);
        let frame = codec.encode(Frame::new(b"some bytes")).unwrap();

        let mut item = Vec::new();
        DeflateDecoder::new(frame.as_item())
            .read_to_end(&mut item)
            .unwrap();
        assert_eq!(item, b"some bytes");

        // Encode using gzip and verify that the compression was as expected by decompressing
        let mut codec = CompressionCodec::gzip(CompressionLevel::BEST);
        let frame = codec.encode(Frame::new(b"some bytes")).unwrap();

        let mut item = Vec::new();
        GzDecoder::new(frame.as_item())
            .read_to_end(&mut item)
            .unwrap();
        assert_eq!(item, b"some bytes");

        // Encode using zlib and verify that the compression was as expected by decompressing
        let mut codec = CompressionCodec::zlib(CompressionLevel::BEST);
        let frame = codec.encode(Frame::new(b"some bytes")).unwrap();

        let mut item = Vec::new();
        ZlibDecoder::new(frame.as_item())
            .read_to_end(&mut item)
            .unwrap();
        assert_eq!(item, b"some bytes");
    }

    #[test]
    fn decode_should_apply_appropriate_decompression_algorithm() {
        // Decode using DEFLATE
        let frame = {
            let mut item = Vec::new();
            DeflateEncoder::new(b"some bytes".as_slice(), Compression::best())
                .read_to_end(&mut item)
                .unwrap();
            Frame::from(item)
        };
        let mut codec = CompressionCodec::deflate(CompressionLevel::BEST);
        let frame = codec.decode(frame).unwrap();
        assert_eq!(frame, b"some bytes");

        // Decode using gzip
        let frame = {
            let mut item = Vec::new();
            GzEncoder::new(b"some bytes".as_slice(), Compression::best())
                .read_to_end(&mut item)
                .unwrap();
            Frame::from(item)
        };
        let mut codec = CompressionCodec::gzip(CompressionLevel::BEST);
        let frame = codec.decode(frame).unwrap();
        assert_eq!(frame, b"some bytes");

        // Decode using zlib
        let frame = {
            let mut item = Vec::new();
            ZlibEncoder::new(b"some bytes".as_slice(), Compression::best())
                .read_to_end(&mut item)
                .unwrap();
            Frame::from(item)
        };
        let mut codec = CompressionCodec::zlib(CompressionLevel::BEST);
        let frame = codec.decode(frame).unwrap();
        assert_eq!(frame, b"some bytes");
    }
}
