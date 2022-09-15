use super::{Codec, Frame};
use crate::{SecretKey, SecretKey32};
use chacha20poly1305::{aead::Aead, Key, KeyInit, XChaCha20Poly1305, XNonce};
use std::{fmt, io};

/// Total bytes to use for nonce
const NONCE_SIZE: usize = 24;

/// Represents the codec that encodes & decodes frames by encrypting/decrypting them using
/// [`XChaCha20Poly1305`].
///
/// NOTE: Uses a 32-byte key internally.
#[derive(Clone)]
pub struct XChaCha20Poly1305Codec {
    cipher: XChaCha20Poly1305,
}

impl XChaCha20Poly1305Codec {
    pub fn new(key: &[u8]) -> Self {
        let key = Key::from_slice(key);
        let cipher = XChaCha20Poly1305::new(key);
        Self { cipher }
    }
}

impl From<SecretKey32> for XChaCha20Poly1305Codec {
    /// Create a new XChaCha20Poly1305 codec with a 32-byte key
    fn from(secret_key: SecretKey32) -> Self {
        Self::new(secret_key.unprotected_as_bytes())
    }
}

impl fmt::Debug for XChaCha20Poly1305Codec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("XChaCha20Poly1305Codec")
            .field("cipher", &"**OMITTED**".to_string())
            .finish()
    }
}

impl Codec for XChaCha20Poly1305Codec {
    fn encode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        // NOTE: As seen in orion, with a 24-bit nonce, it's safe to generate instead of
        //       maintaining a stateful counter due to its size (24-byte secret key generation
        //       will never panic)
        let nonce_key = SecretKey::<NONCE_SIZE>::generate().unwrap();
        let nonce = XNonce::from_slice(nonce_key.unprotected_as_bytes());

        // Encrypt the frame's item as our ciphertext
        let ciphertext = self
            .cipher
            .encrypt(nonce, frame.as_item())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Encryption failed"))?;

        // Frame is now comprised of the nonce and ciphertext in sequence
        let mut frame = Frame::new(nonce.as_slice());
        frame.extend(ciphertext);

        Ok(frame.into_owned())
    }

    fn decode<'a>(&mut self, frame: Frame<'a>) -> io::Result<Frame<'a>> {
        if frame.len() <= NONCE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Frame cannot have length less than {}", frame.len()),
            ));
        }

        // Grab the nonce from the front of the frame, and then use it with the remainder
        // of the frame to tease out the decrypted frame item
        let nonce = XNonce::from_slice(&frame.as_item()[..NONCE_SIZE]);
        let item = self
            .cipher
            .decrypt(nonce, &frame.as_item()[NONCE_SIZE..])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Decryption failed"))?;

        Ok(Frame::from(item))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_should_build_a_frame_containing_a_length_nonce_and_ciphertext() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key.clone());

        let frame = codec
            .encode(Frame::new(b"hello world"))
            .expect("Failed to encode");

        let nonce = XNonce::from_slice(&frame.as_item()[..NONCE_SIZE]);
        let ciphertext = &frame.as_item()[NONCE_SIZE..];

        // Manually build our key & cipher so we can decrypt the frame manually to ensure it is
        // correct
        let key = Key::from_slice(key.unprotected_as_bytes());
        let cipher = XChaCha20Poly1305::new(key);
        let item = cipher
            .decrypt(nonce, ciphertext)
            .expect("Failed to decrypt");
        assert_eq!(item, b"hello world");
    }

    #[test]
    fn decode_should_fail_if_frame_length_is_smaller_than_nonce_plus_data() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        // NONCE_SIZE + 1 is minimum for frame length
        let frame = Frame::from(b"a".repeat(NONCE_SIZE));

        let result = codec.decode(frame);
        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn decode_should_fail_if_unable_to_decrypt_frame_item() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        // NONCE_SIZE + 1 is minimum for frame length
        let frame = Frame::from(b"a".repeat(NONCE_SIZE + 1));

        let result = codec.decode(frame);
        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn decode_should_return_decrypted_frame_when_successful() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        let frame = codec
            .encode(Frame::new(b"hello, world"))
            .expect("Failed to encode");

        let frame = codec.decode(frame).expect("Failed to decode");
        assert_eq!(frame, b"hello, world");
    }
}
