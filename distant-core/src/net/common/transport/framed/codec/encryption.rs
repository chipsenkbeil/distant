use std::{fmt, io};

use derive_more::Display;

use super::{Codec, Frame};
use crate::net::common::{SecretKey, SecretKey32};

/// Represents the type of encryption for a [`EncryptionCodec`]
#[derive(
    Copy, Clone, Debug, Display, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum EncryptionType {
    /// ChaCha20Poly1305 variant with an extended 192-bit (24-byte) nonce
    #[display(fmt = "xchacha20poly1305")]
    XChaCha20Poly1305,

    /// Indicates an unknown encryption type for use in handshakes
    #[display(fmt = "unknown")]
    #[serde(other)]
    Unknown,
}

impl EncryptionType {
    /// Generates bytes for a secret key based on the encryption type
    pub fn generate_secret_key_bytes(&self) -> io::Result<Vec<u8>> {
        match self {
            Self::XChaCha20Poly1305 => Ok(SecretKey::<32>::generate()
                .unwrap()
                .into_heap_secret_key()
                .unprotected_into_bytes()),
            Self::Unknown => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unknown encryption type",
            )),
        }
    }

    /// Returns a list of all variants of the type *except* unknown.
    pub const fn known_variants() -> &'static [EncryptionType] {
        &[EncryptionType::XChaCha20Poly1305]
    }

    /// Returns true if type is unknown
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// Creates a new [`EncryptionCodec`] for this type, failing if this type is unknown or the key
    /// is an invalid length
    pub fn new_codec(&self, key: &[u8]) -> io::Result<EncryptionCodec> {
        EncryptionCodec::from_type_and_key(*self, key)
    }
}

/// Represents the codec that encodes & decodes frames by encrypting/decrypting them
#[derive(Clone)]
pub enum EncryptionCodec {
    /// ChaCha20Poly1305 variant with an extended 192-bit (24-byte) nonce, using
    /// [`XChaCha20Poly1305`] underneath
    XChaCha20Poly1305 {
        cipher: chacha20poly1305::XChaCha20Poly1305,
    },
}

impl EncryptionCodec {
    /// Makes a new [`EncryptionCodec`] based on the [`EncryptionType`] and `key`, returning an
    /// error if the key is invalid for the encryption type or the type is unknown
    pub fn from_type_and_key(ty: EncryptionType, key: &[u8]) -> io::Result<EncryptionCodec> {
        match ty {
            EncryptionType::XChaCha20Poly1305 => {
                use chacha20poly1305::{KeyInit, XChaCha20Poly1305};
                let cipher = XChaCha20Poly1305::new_from_slice(key)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?;
                Ok(Self::XChaCha20Poly1305 { cipher })
            }
            EncryptionType::Unknown => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Encryption type is unknown",
            )),
        }
    }

    pub fn new_xchacha20poly1305(secret_key: SecretKey32) -> EncryptionCodec {
        // NOTE: This should never fail as we are enforcing the key size at compile time
        Self::from_type_and_key(
            EncryptionType::XChaCha20Poly1305,
            secret_key.unprotected_as_bytes(),
        )
        .unwrap()
    }

    /// Returns the encryption type associa ted with the codec
    pub fn ty(&self) -> EncryptionType {
        match self {
            Self::XChaCha20Poly1305 { .. } => EncryptionType::XChaCha20Poly1305,
        }
    }

    /// Size of nonce (in bytes) associated with the encryption algorithm
    pub const fn nonce_size(&self) -> usize {
        match self {
            // XChaCha20Poly1305 uses a 192-bit (24-byte) key
            Self::XChaCha20Poly1305 { .. } => 24,
        }
    }

    /// Generates a new nonce for the encryption algorithm
    fn generate_nonce_bytes(&self) -> Vec<u8> {
        // NOTE: As seen in orion, with a 24-bit nonce, it's safe to generate instead of
        //       maintaining a stateful counter due to its size (24-byte secret key generation
        //       will never panic)
        match self {
            Self::XChaCha20Poly1305 { .. } => SecretKey::<24>::generate()
                .unwrap()
                .into_heap_secret_key()
                .unprotected_into_bytes(),
        }
    }
}

impl fmt::Debug for EncryptionCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptionCodec")
            .field("cipher", &"**OMITTED**".to_string())
            .field("nonce_size", &self.nonce_size())
            .field("ty", &self.ty().to_string())
            .finish()
    }
}

impl Codec for EncryptionCodec {
    fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        let nonce_bytes = self.generate_nonce_bytes();

        Ok(match self {
            Self::XChaCha20Poly1305 { cipher } => {
                use chacha20poly1305::XNonce;
                use chacha20poly1305::aead::Aead;
                let item = frame.into_item();
                let nonce = XNonce::from_slice(&nonce_bytes);

                // Encrypt the frame's item as our ciphertext
                let ciphertext = cipher
                    .encrypt(nonce, item.as_ref())
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Encryption failed"))?;

                // Start our frame with the nonce at the beginning
                let mut frame = Frame::from(nonce_bytes);
                frame.extend(ciphertext);

                frame
            }
        })
    }

    fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        let nonce_size = self.nonce_size();
        if frame.len() <= nonce_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Frame cannot have length less than {}", nonce_size + 1),
            ));
        }

        // Grab the nonce from the front of the frame, and then use it with the remainder
        // of the frame to tease out the decrypted frame item
        let item = match self {
            Self::XChaCha20Poly1305 { cipher } => {
                use chacha20poly1305::XNonce;
                use chacha20poly1305::aead::Aead;
                let nonce = XNonce::from_slice(&frame.as_item()[..nonce_size]);
                cipher
                    .decrypt(nonce, &frame.as_item()[nonce_size..])
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Decryption failed"))?
            }
        };

        Ok(Frame::from(item))
    }
}

#[cfg(test)]
mod tests {
    //! Tests for EncryptionType and EncryptionCodec: key generation, codec construction,
    //! encode/decode round-trips, nonce randomness, wrong-key rejection, edge cases
    //! (empty frame, nonce-only frame), Debug redaction, and Clone independence.

    use test_log::test;

    use super::*;

    #[test]
    fn encode_should_build_a_frame_containing_a_length_nonce_and_ciphertext() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        let frame = codec
            .encode(Frame::new(b"hello world"))
            .expect("Failed to encode");

        let nonce = &frame.as_item()[..codec.nonce_size()];
        let ciphertext = &frame.as_item()[codec.nonce_size()..];

        // Manually build our key & cipher so we can decrypt the frame manually to ensure it is
        // correct
        let item = {
            use chacha20poly1305::aead::Aead;
            use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
            let cipher = XChaCha20Poly1305::new_from_slice(&key).unwrap();
            cipher
                .decrypt(XNonce::from_slice(nonce), ciphertext)
                .expect("Failed to decrypt")
        };
        assert_eq!(item, b"hello world");
    }

    #[test]
    fn decode_should_fail_if_frame_length_is_smaller_than_nonce_plus_data() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        // NONCE_SIZE + 1 is minimum for frame length
        let frame = Frame::from(b"a".repeat(codec.nonce_size()));

        let result = codec.decode(frame);
        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn decode_should_fail_if_unable_to_decrypt_frame_item() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        // NONCE_SIZE + 1 is minimum for frame length
        let frame = Frame::from(b"a".repeat(codec.nonce_size() + 1));

        let result = codec.decode(frame);
        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn decode_should_return_decrypted_frame_when_successful() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        let frame = codec
            .encode(Frame::new(b"hello, world"))
            .expect("Failed to encode");

        let frame = codec.decode(frame).expect("Failed to decode");
        assert_eq!(frame, b"hello, world");
    }

    // --- EncryptionType tests ---

    #[test]
    fn encryption_type_display_xchacha20poly1305() {
        assert_eq!(
            EncryptionType::XChaCha20Poly1305.to_string(),
            "xchacha20poly1305"
        );
    }

    #[test]
    fn encryption_type_display_unknown() {
        assert_eq!(EncryptionType::Unknown.to_string(), "unknown");
    }

    #[test]
    fn encryption_type_known_variants_should_not_include_unknown() {
        let variants = EncryptionType::known_variants();
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0], EncryptionType::XChaCha20Poly1305);
        assert!(!variants.iter().any(|v| v.is_unknown()));
    }

    #[test]
    fn encryption_type_is_unknown_should_return_true_for_unknown() {
        assert!(EncryptionType::Unknown.is_unknown());
    }

    #[test]
    fn encryption_type_is_unknown_should_return_false_for_xchacha20poly1305() {
        assert!(!EncryptionType::XChaCha20Poly1305.is_unknown());
    }

    #[test]
    fn encryption_type_generate_secret_key_bytes_should_produce_32_bytes_for_xchacha20() {
        let bytes = EncryptionType::XChaCha20Poly1305
            .generate_secret_key_bytes()
            .unwrap();
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn encryption_type_generate_secret_key_bytes_should_fail_for_unknown() {
        let result = EncryptionType::Unknown.generate_secret_key_bytes();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn encryption_type_generate_secret_key_bytes_should_produce_unique_keys() {
        let key1 = EncryptionType::XChaCha20Poly1305
            .generate_secret_key_bytes()
            .unwrap();
        let key2 = EncryptionType::XChaCha20Poly1305
            .generate_secret_key_bytes()
            .unwrap();
        assert_ne!(key1, key2, "Two generated keys should differ");
    }

    #[test]
    fn encryption_type_new_codec_should_succeed_for_xchacha20() {
        let key = EncryptionType::XChaCha20Poly1305
            .generate_secret_key_bytes()
            .unwrap();
        let result = EncryptionType::XChaCha20Poly1305.new_codec(&key);
        assert!(result.is_ok());
    }

    #[test]
    fn encryption_type_new_codec_should_fail_for_unknown() {
        let result = EncryptionType::Unknown.new_codec(&[0u8; 32]);
        assert!(result.is_err());
    }

    #[test]
    fn encryption_type_serde_round_trip_xchacha20poly1305() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let json = serde_json::to_string(&ty).unwrap();
        let deserialized: EncryptionType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ty);
    }

    #[test]
    fn encryption_type_serde_unknown_variant_should_deserialize_as_unknown() {
        let json = r#""SomeNewType""#;
        let deserialized: EncryptionType = serde_json::from_str(json).unwrap();
        assert_eq!(deserialized, EncryptionType::Unknown);
    }

    // --- EncryptionCodec construction tests ---

    #[test]
    fn from_type_and_key_should_fail_for_unknown_type() {
        let result = EncryptionCodec::from_type_and_key(EncryptionType::Unknown, &[0u8; 32]);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn from_type_and_key_should_fail_for_invalid_key_length() {
        // XChaCha20Poly1305 requires 32 bytes; provide 16
        let result =
            EncryptionCodec::from_type_and_key(EncryptionType::XChaCha20Poly1305, &[0u8; 16]);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn from_type_and_key_should_fail_for_empty_key() {
        let result = EncryptionCodec::from_type_and_key(EncryptionType::XChaCha20Poly1305, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn new_xchacha20poly1305_should_create_codec() {
        let secret_key = SecretKey32::generate().unwrap();
        let codec = EncryptionCodec::new_xchacha20poly1305(secret_key);
        assert_eq!(codec.ty(), EncryptionType::XChaCha20Poly1305);
    }

    // --- EncryptionCodec accessor tests ---

    #[test]
    fn ty_should_return_xchacha20poly1305_for_xchacha20_codec() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();
        assert_eq!(codec.ty(), EncryptionType::XChaCha20Poly1305);
    }

    #[test]
    fn nonce_size_should_return_24_for_xchacha20poly1305() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();
        assert_eq!(codec.nonce_size(), 24);
    }

    // --- EncryptionCodec Debug ---

    #[test]
    fn debug_should_omit_cipher_details() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();
        let debug_str = format!("{:?}", codec);
        assert!(debug_str.contains("**OMITTED**"));
        assert!(debug_str.contains("nonce_size"));
        assert!(debug_str.contains("24"));
        assert!(debug_str.contains("xchacha20poly1305"));
    }

    // --- EncryptionCodec Clone ---

    #[test]
    fn clone_should_produce_independently_usable_codec() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();
        let mut cloned = codec.clone();

        // Encode with original, decode with clone
        let frame = codec
            .encode(Frame::new(b"test clone"))
            .expect("Failed to encode");
        let decoded = cloned.decode(frame).expect("Failed to decode");
        assert_eq!(decoded, b"test clone");
    }

    // --- Encode/Decode edge cases ---

    #[test]
    fn encode_then_decode_single_byte_should_round_trip() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        let frame = codec.encode(Frame::new(&[42])).expect("Failed to encode");
        let frame = codec.decode(frame).expect("Failed to decode");
        assert_eq!(frame, [42]);
    }

    #[test]
    fn encode_then_decode_large_data_should_round_trip() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        let data: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();
        let frame = codec.encode(Frame::new(&data)).expect("Failed to encode");
        let frame = codec.decode(frame).expect("Failed to decode");
        assert_eq!(frame.as_item(), data.as_slice());
    }

    #[test]
    fn decode_should_fail_if_frame_is_empty() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        let result = codec.decode(Frame::empty());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn decode_should_fail_if_frame_is_exactly_nonce_size() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        // Exactly nonce_size bytes means no ciphertext at all
        let frame = Frame::from(vec![0u8; codec.nonce_size()]);
        let result = codec.decode(frame);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn decode_with_wrong_key_should_fail() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key1 = ty.generate_secret_key_bytes().unwrap();
        let key2 = ty.generate_secret_key_bytes().unwrap();

        let mut encoder = EncryptionCodec::from_type_and_key(ty, &key1).unwrap();
        let mut decoder = EncryptionCodec::from_type_and_key(ty, &key2).unwrap();

        let frame = encoder
            .encode(Frame::new(b"secret message"))
            .expect("Failed to encode");

        let result = decoder.decode(frame);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn encode_produces_different_ciphertext_each_time_due_to_random_nonce() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        let frame1 = codec
            .encode(Frame::new(b"same input"))
            .expect("Failed to encode");
        let frame2 = codec
            .encode(Frame::new(b"same input"))
            .expect("Failed to encode");

        // The nonces should differ, making the ciphertext different
        assert_ne!(frame1.as_item(), frame2.as_item());
    }

    #[test]
    fn encoded_frame_should_be_larger_than_original_due_to_nonce_and_auth_tag() {
        let ty = EncryptionType::XChaCha20Poly1305;
        let key = ty.generate_secret_key_bytes().unwrap();
        let mut codec = EncryptionCodec::from_type_and_key(ty, &key).unwrap();

        let original = Frame::new(b"hello");
        let encoded = codec.encode(original).expect("Failed to encode");

        // Encoded should be: nonce (24) + plaintext (5) + auth tag (16) = 45
        assert!(
            encoded.len() > 5,
            "Encoded frame should be larger than original"
        );
        assert_eq!(encoded.len(), 24 + 5 + 16);
    }
}
