use crate::net::{Codec, SecretKey, SecretKey32};
use bytes::{Buf, BufMut, BytesMut};
use chacha20poly1305::{
    aead::{Aead, NewAead},
    Key, XChaCha20Poly1305, XNonce,
};
use std::{convert::TryInto, fmt};
use tokio::io;

/// Total bytes to use as the len field denoting a frame's size
const LEN_SIZE: usize = 8;

/// Total bytes to use for nonce
const NONCE_SIZE: usize = 24;

/// Represents the codec to encode & decode data while also encrypting/decrypting it
///
/// Uses a 32-byte key internally
#[derive(Clone)]
pub struct XChaCha20Poly1305Codec {
    cipher: XChaCha20Poly1305,
}
impl_traits_for_codec!(XChaCha20Poly1305Codec);

impl From<SecretKey32> for XChaCha20Poly1305Codec {
    /// Create a new XChaCha20Poly1305 codec with a 32-byte key
    fn from(secret_key: SecretKey32) -> Self {
        let key = Key::from_slice(secret_key.unprotected_as_bytes());
        let cipher = XChaCha20Poly1305::new(key);
        Self { cipher }
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
    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> io::Result<()> {
        // Validate that we can fit the message plus nonce +
        if item.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Empty item provided",
            ));
        }
        // NOTE: As seen in orion, with a 24-bit nonce, it's safe to generate instead of
        //       maintaining a stateful counter due to its size (24-byte secret key generation
        //       will never panic)
        let nonce_key = SecretKey::<NONCE_SIZE>::generate().unwrap();
        let nonce = XNonce::from_slice(nonce_key.unprotected_as_bytes());

        let ciphertext = self
            .cipher
            .encrypt(nonce, item)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Encryption failed"))?;

        dst.reserve(8 + nonce.len() + ciphertext.len());

        // Add data in form of {LEN}{NONCE}{CIPHER TEXT}
        dst.put_u64((nonce_key.len() + ciphertext.len()) as u64);
        dst.put_slice(nonce.as_slice());
        dst.extend(ciphertext);

        Ok(())
    }

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Vec<u8>>> {
        // First, check if we have more data than just our frame's message length
        if src.len() <= LEN_SIZE {
            return Ok(None);
        }

        // Second, retrieve total size of our frame's message
        let msg_len = u64::from_be_bytes(src[..LEN_SIZE].try_into().unwrap()) as usize;
        if msg_len <= NONCE_SIZE {
            // Ensure we advance to remove the frame
            src.advance(LEN_SIZE + msg_len);

            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Frame's msg cannot have length less than 25",
            ));
        }

        // Third, check if we have all data for our frame; if not, exit early
        if src.len() < msg_len + LEN_SIZE {
            return Ok(None);
        }

        // Fourth, retrieve the nonce used with the ciphertext
        let nonce = XNonce::from_slice(&src[LEN_SIZE..(NONCE_SIZE + LEN_SIZE)]);

        // Fifth, acquire the encrypted & signed ciphertext
        let ciphertext = &src[(NONCE_SIZE + LEN_SIZE)..(msg_len + LEN_SIZE)];

        // Sixth, convert ciphertext back into our item
        let item = self.cipher.decrypt(nonce, ciphertext);

        // Seventh, advance so frame is no longer kept around
        src.advance(LEN_SIZE + msg_len);

        // Eighth, report an error if there is one
        let item =
            item.map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Decryption failed"))?;

        Ok(Some(item))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_should_fail_when_item_is_zero_bytes() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        let mut buf = BytesMut::new();
        let result = codec.encode(&[], &mut buf);

        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidInput => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn encode_should_build_a_frame_containing_a_length_nonce_and_ciphertext() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        let mut buf = BytesMut::new();
        codec
            .encode(b"hello, world", &mut buf)
            .expect("Failed to encode");

        let len = buf.get_u64() as usize;
        assert!(buf.len() > NONCE_SIZE, "Msg size not big enough");
        assert_eq!(len, buf.len(), "Msg size does not match attached size");
    }

    #[test]
    fn decode_should_return_none_if_data_smaller_than_or_equal_to_frame_length_field() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        let mut buf = BytesMut::new();
        buf.put_bytes(0, LEN_SIZE);

        let result = codec.decode(&mut buf);
        assert!(
            matches!(result, Ok(None)),
            "Unexpected result: {:?}",
            result
        );
    }

    #[test]
    fn decode_should_return_none_if_not_enough_data_for_frame() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        let mut buf = BytesMut::new();
        buf.put_u64(0);

        let result = codec.decode(&mut buf);
        assert!(
            matches!(result, Ok(None)),
            "Unexpected result: {:?}",
            result
        );
    }

    #[test]
    fn decode_should_fail_if_encoded_frame_length_is_smaller_than_nonce_plus_data() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        // NONCE_SIZE + 1 is minimum for frame length
        let mut buf = BytesMut::new();
        buf.put_u64(NONCE_SIZE as u64);
        buf.put_bytes(0, NONCE_SIZE);

        let result = codec.decode(&mut buf);
        match result {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test]
    fn decode_should_advance_src_by_frame_size_even_if_frame_length_is_too_small() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        // LEN_SIZE + NONCE_SIZE + msg not matching encryption + 3 more bytes
        let mut buf = BytesMut::new();
        buf.put_u64(NONCE_SIZE as u64);
        buf.put_bytes(0, NONCE_SIZE);
        buf.put_bytes(0, 3);

        assert!(
            codec.decode(&mut buf).is_err(),
            "Decode unexpectedly succeeded"
        );
        assert_eq!(buf.len(), 3, "Advanced an unexpected amount in src buf");
    }

    #[test]
    fn decode_should_advance_src_by_frame_size_even_if_decryption_fails() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        // LEN_SIZE + NONCE_SIZE + msg not matching encryption + 3 more bytes
        let mut buf = BytesMut::new();
        buf.put_u64((NONCE_SIZE + 12) as u64);
        buf.put_bytes(0, NONCE_SIZE);
        buf.put_slice(b"hello, world");
        buf.put_bytes(0, 3);

        assert!(
            codec.decode(&mut buf).is_err(),
            "Decode unexpectedly succeeded"
        );
        assert_eq!(buf.len(), 3, "Advanced an unexpected amount in src buf");
    }

    #[test]
    fn decode_should_advance_src_by_frame_size_when_successful() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        // Add 3 extra bytes after a full frame
        let mut buf = BytesMut::new();
        codec
            .encode(b"hello, world", &mut buf)
            .expect("Failed to encode");
        buf.put_bytes(0, 3);

        assert!(codec.decode(&mut buf).is_ok(), "Decode unexpectedly failed");
        assert_eq!(buf.len(), 3, "Advanced an unexpected amount in src buf");
    }

    #[test]
    fn decode_should_return_some_byte_vec_when_successful() {
        let key = SecretKey32::default();
        let mut codec = XChaCha20Poly1305Codec::from(key);

        let mut buf = BytesMut::new();
        codec
            .encode(b"hello, world", &mut buf)
            .expect("Failed to encode");

        let item = codec
            .decode(&mut buf)
            .expect("Failed to decode")
            .expect("Item not properly captured");
        assert_eq!(item, b"hello, world");
    }
}
