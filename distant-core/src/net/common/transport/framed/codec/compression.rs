use std::io::{self, Read};

use flate2::bufread::{
    DeflateDecoder, DeflateEncoder, GzDecoder, GzEncoder, ZlibDecoder, ZlibEncoder,
};
use flate2::Compression;
use serde::{Deserialize, Serialize};

use super::{Codec, Frame};

/// Represents the level of compression to apply to data
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
    /// Applies best compression to reduce size (slowest)
    pub const BEST: Self = Self::Nine;
    /// Applies fastest compression
    pub const FAST: Self = Self::One;
    /// Applies no compression
    pub const NONE: Self = Self::Zero;
}

impl Default for CompressionLevel {
    /// Standard compression level used in zlib library is 6, which is also used here
    fn default() -> Self {
        Self::Six
    }
}

/// Represents the type of compression for a [`CompressionCodec`]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompressionType {
    Deflate,
    Gzip,
    Zlib,

    /// Indicates an unknown compression type for use in handshakes
    #[serde(other)]
    Unknown,
}

impl CompressionType {
    /// Returns a list of all variants of the type *except* unknown.
    pub const fn known_variants() -> &'static [CompressionType] {
        &[
            CompressionType::Deflate,
            CompressionType::Gzip,
            CompressionType::Zlib,
        ]
    }

    /// Returns true if type is unknown
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// Creates a new [`CompressionCodec`] for this type, failing if this type is unknown
    pub fn new_codec(&self, level: CompressionLevel) -> io::Result<CompressionCodec> {
        CompressionCodec::from_type_and_level(*self, level)
    }
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
    /// Makes a new [`CompressionCodec`] based on the [`CompressionType`] and [`CompressionLevel`],
    /// returning error if the type is unknown
    pub fn from_type_and_level(
        ty: CompressionType,
        level: CompressionLevel,
    ) -> io::Result<CompressionCodec> {
        match ty {
            CompressionType::Deflate => Ok(Self::Deflate { level }),
            CompressionType::Gzip => Ok(Self::Gzip { level }),
            CompressionType::Zlib => Ok(Self::Zlib { level }),
            CompressionType::Unknown => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unknown compression type",
            )),
        }
    }

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

    /// Returns the compression type associated with the codec
    pub fn ty(&self) -> CompressionType {
        match self {
            Self::Deflate { .. } => CompressionType::Deflate,
            Self::Gzip { .. } => CompressionType::Gzip,
            Self::Zlib { .. } => CompressionType::Zlib,
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
    use test_log::test;

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

    // -----------------------------------------------------------------------
    // CompressionLevel
    // -----------------------------------------------------------------------

    #[test]
    fn compression_level_default_is_six() {
        assert_eq!(CompressionLevel::default(), CompressionLevel::Six);
    }

    #[test]
    fn compression_level_constants() {
        assert_eq!(CompressionLevel::BEST, CompressionLevel::Nine);
        assert_eq!(CompressionLevel::FAST, CompressionLevel::One);
        assert_eq!(CompressionLevel::NONE, CompressionLevel::Zero);
    }

    #[test]
    fn compression_level_ordering() {
        assert!(CompressionLevel::Zero < CompressionLevel::One);
        assert!(CompressionLevel::One < CompressionLevel::Nine);
        assert!(CompressionLevel::NONE < CompressionLevel::FAST);
        assert!(CompressionLevel::FAST < CompressionLevel::BEST);
    }

    #[test]
    fn compression_level_serde_roundtrip() {
        let level = CompressionLevel::Seven;
        let json = serde_json::to_string(&level).unwrap();
        let restored: CompressionLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(level, restored);
    }

    // -----------------------------------------------------------------------
    // CompressionType
    // -----------------------------------------------------------------------

    #[test]
    fn known_variants_returns_all_non_unknown_types() {
        let variants = CompressionType::known_variants();
        assert_eq!(variants.len(), 3);
        assert!(variants.contains(&CompressionType::Deflate));
        assert!(variants.contains(&CompressionType::Gzip));
        assert!(variants.contains(&CompressionType::Zlib));
    }

    #[test]
    fn is_unknown_returns_true_only_for_unknown() {
        assert!(CompressionType::Unknown.is_unknown());
        assert!(!CompressionType::Deflate.is_unknown());
        assert!(!CompressionType::Gzip.is_unknown());
        assert!(!CompressionType::Zlib.is_unknown());
    }

    #[test]
    fn new_codec_creates_codec_for_known_types() {
        let codec = CompressionType::Deflate
            .new_codec(CompressionLevel::FAST)
            .unwrap();
        assert_eq!(codec.ty(), CompressionType::Deflate);
        assert_eq!(codec.level(), CompressionLevel::FAST);

        let codec = CompressionType::Gzip
            .new_codec(CompressionLevel::BEST)
            .unwrap();
        assert_eq!(codec.ty(), CompressionType::Gzip);
        assert_eq!(codec.level(), CompressionLevel::BEST);

        let codec = CompressionType::Zlib
            .new_codec(CompressionLevel::default())
            .unwrap();
        assert_eq!(codec.ty(), CompressionType::Zlib);
        assert_eq!(codec.level(), CompressionLevel::Six);
    }

    #[test]
    fn new_codec_fails_for_unknown_type() {
        let result = CompressionType::Unknown.new_codec(CompressionLevel::FAST);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn compression_type_serde_roundtrip() {
        for ty in CompressionType::known_variants() {
            let json = serde_json::to_string(ty).unwrap();
            let restored: CompressionType = serde_json::from_str(&json).unwrap();
            assert_eq!(*ty, restored);
        }
    }

    #[test]
    fn compression_type_unknown_deserializes_from_unrecognized_string() {
        let ty: CompressionType = serde_json::from_str("\"SomeNewType\"").unwrap();
        assert!(ty.is_unknown());
    }

    // -----------------------------------------------------------------------
    // CompressionCodec constructor helpers
    // -----------------------------------------------------------------------

    #[test]
    fn deflate_constructor() {
        let codec = CompressionCodec::deflate(CompressionLevel::Three);
        assert_eq!(codec.ty(), CompressionType::Deflate);
        assert_eq!(codec.level(), CompressionLevel::Three);
    }

    #[test]
    fn gzip_constructor() {
        let codec = CompressionCodec::gzip(CompressionLevel::Five);
        assert_eq!(codec.ty(), CompressionType::Gzip);
        assert_eq!(codec.level(), CompressionLevel::Five);
    }

    #[test]
    fn zlib_constructor() {
        let codec = CompressionCodec::zlib(CompressionLevel::Eight);
        assert_eq!(codec.ty(), CompressionType::Zlib);
        assert_eq!(codec.level(), CompressionLevel::Eight);
    }

    // -----------------------------------------------------------------------
    // from_type_and_level
    // -----------------------------------------------------------------------

    #[test]
    fn from_type_and_level_succeeds_for_all_known_types() {
        let level = CompressionLevel::Four;
        let deflate =
            CompressionCodec::from_type_and_level(CompressionType::Deflate, level).unwrap();
        assert_eq!(deflate, CompressionCodec::Deflate { level });

        let gzip = CompressionCodec::from_type_and_level(CompressionType::Gzip, level).unwrap();
        assert_eq!(gzip, CompressionCodec::Gzip { level });

        let zlib = CompressionCodec::from_type_and_level(CompressionType::Zlib, level).unwrap();
        assert_eq!(zlib, CompressionCodec::Zlib { level });
    }

    #[test]
    fn from_type_and_level_fails_for_unknown() {
        let result =
            CompressionCodec::from_type_and_level(CompressionType::Unknown, CompressionLevel::BEST);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("Unknown compression type"));
    }

    // -----------------------------------------------------------------------
    // level() and ty() accessors
    // -----------------------------------------------------------------------

    #[test]
    fn level_returns_correct_level_for_each_variant() {
        assert_eq!(
            CompressionCodec::Deflate {
                level: CompressionLevel::Two
            }
            .level(),
            CompressionLevel::Two
        );
        assert_eq!(
            CompressionCodec::Gzip {
                level: CompressionLevel::Seven
            }
            .level(),
            CompressionLevel::Seven
        );
        assert_eq!(
            CompressionCodec::Zlib {
                level: CompressionLevel::Nine
            }
            .level(),
            CompressionLevel::Nine
        );
    }

    #[test]
    fn ty_returns_correct_type_for_each_variant() {
        assert_eq!(
            CompressionCodec::Deflate {
                level: CompressionLevel::NONE
            }
            .ty(),
            CompressionType::Deflate
        );
        assert_eq!(
            CompressionCodec::Gzip {
                level: CompressionLevel::NONE
            }
            .ty(),
            CompressionType::Gzip
        );
        assert_eq!(
            CompressionCodec::Zlib {
                level: CompressionLevel::NONE
            }
            .ty(),
            CompressionType::Zlib
        );
    }

    // -----------------------------------------------------------------------
    // Empty data encode/decode
    // -----------------------------------------------------------------------

    #[test]
    fn encode_decode_empty_data_deflate() {
        let mut codec = CompressionCodec::deflate(CompressionLevel::default());
        let encoded = codec.encode(Frame::new(b"")).unwrap();
        let decoded = codec.decode(encoded).unwrap();
        assert_eq!(decoded.as_item(), b"");
    }

    #[test]
    fn encode_decode_empty_data_gzip() {
        let mut codec = CompressionCodec::gzip(CompressionLevel::default());
        let encoded = codec.encode(Frame::new(b"")).unwrap();
        let decoded = codec.decode(encoded).unwrap();
        assert_eq!(decoded.as_item(), b"");
    }

    #[test]
    fn encode_decode_empty_data_zlib() {
        let mut codec = CompressionCodec::zlib(CompressionLevel::default());
        let encoded = codec.encode(Frame::new(b"")).unwrap();
        let decoded = codec.decode(encoded).unwrap();
        assert_eq!(decoded.as_item(), b"");
    }

    // -----------------------------------------------------------------------
    // Large data encode/decode round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn encode_decode_large_data_roundtrip() {
        let large_data: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();

        for mut codec in [
            CompressionCodec::deflate(CompressionLevel::BEST),
            CompressionCodec::gzip(CompressionLevel::BEST),
            CompressionCodec::zlib(CompressionLevel::BEST),
        ] {
            let encoded = codec.encode(Frame::new(&large_data)).unwrap();
            // Compressed data should typically be smaller
            assert!(
                encoded.as_item().len() < large_data.len(),
                "Expected compressed size < original for {:?}",
                codec.ty()
            );
            let decoded = codec.decode(encoded).unwrap();
            assert_eq!(decoded.as_item(), large_data.as_slice());
        }
    }

    // -----------------------------------------------------------------------
    // Different compression levels produce valid output
    // -----------------------------------------------------------------------

    #[test]
    fn all_compression_levels_produce_decodable_output() {
        let data = b"The quick brown fox jumps over the lazy dog";
        let levels = [
            CompressionLevel::Zero,
            CompressionLevel::One,
            CompressionLevel::Two,
            CompressionLevel::Three,
            CompressionLevel::Four,
            CompressionLevel::Five,
            CompressionLevel::Six,
            CompressionLevel::Seven,
            CompressionLevel::Eight,
            CompressionLevel::Nine,
        ];

        for level in levels {
            let mut codec = CompressionCodec::deflate(level);
            let encoded = codec.encode(Frame::new(data)).unwrap();
            let decoded = codec.decode(encoded).unwrap();
            assert_eq!(decoded.as_item(), data);
        }
    }

    // -----------------------------------------------------------------------
    // Decode with corrupted data should fail
    // -----------------------------------------------------------------------

    #[test]
    fn decode_corrupted_data_should_fail_deflate() {
        let mut codec = CompressionCodec::deflate(CompressionLevel::default());
        let result = codec.decode(Frame::new(b"this is not compressed"));
        assert!(result.is_err());
    }

    #[test]
    fn decode_corrupted_data_should_fail_gzip() {
        let mut codec = CompressionCodec::gzip(CompressionLevel::default());
        let result = codec.decode(Frame::new(b"this is not compressed"));
        assert!(result.is_err());
    }

    #[test]
    fn decode_corrupted_data_should_fail_zlib() {
        let mut codec = CompressionCodec::zlib(CompressionLevel::default());
        let result = codec.decode(Frame::new(b"this is not compressed"));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Clone and Copy
    // -----------------------------------------------------------------------

    #[test]
    fn compression_codec_is_copy() {
        let codec = CompressionCodec::gzip(CompressionLevel::FAST);
        let copied = codec;
        // Both should still be usable since CompressionCodec is Copy
        assert_eq!(codec.level(), copied.level());
        assert_eq!(codec.ty(), copied.ty());
    }
}
