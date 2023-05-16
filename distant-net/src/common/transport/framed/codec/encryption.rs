use std::{fmt, io};

use derive_more::Display;

use super::{Codec, Frame};

mod key;
pub use key::*;

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
                use chacha20poly1305::aead::Aead;
                use chacha20poly1305::XNonce;
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
                use chacha20poly1305::aead::Aead;
                use chacha20poly1305::XNonce;
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
}
