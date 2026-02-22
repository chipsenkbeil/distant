use std::fmt;
use std::str::FromStr;

use derive_more::{Display, Error};
use rand::rngs::OsRng;
use rand::RngCore;

#[derive(Debug, Display, Error)]
pub struct SecretKeyError;

impl From<SecretKeyError> for std::io::Error {
    fn from(_: SecretKeyError) -> Self {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not valid secret key format",
        )
    }
}

/// Represents a 16-byte (128-bit) secret key
pub type SecretKey16 = SecretKey<16>;

/// Represents a 24-byte (192-bit) secret key
pub type SecretKey24 = SecretKey<24>;

/// Represents a 32-byte (256-bit) secret key
pub type SecretKey32 = SecretKey<32>;

/// Represents a secret key used with transport encryption and authentication
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey<const N: usize>([u8; N]);

impl<const N: usize> fmt::Debug for SecretKey<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("SecretKey")
            .field(&"**OMITTED**".to_string())
            .finish()
    }
}

impl<const N: usize> Default for SecretKey<N> {
    /// Creates a new secret key of the size `N`
    ///
    /// ### Panic
    ///
    /// Will panic if `N` is less than 1 or greater than `isize::MAX`
    fn default() -> Self {
        Self::generate().unwrap()
    }
}

impl<const N: usize> SecretKey<N> {
    /// Returns byte slice to the key's bytes
    pub fn unprotected_as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns reference to array of key's bytes
    pub fn unprotected_as_byte_array(&self) -> &[u8; N] {
        &self.0
    }

    /// Consumes the secret key and returns the array of key's bytes
    pub fn unprotected_into_byte_array(self) -> [u8; N] {
        self.0
    }

    /// Consumes the secret key and returns the key's bytes as a [`HeapSecretKey`]
    pub fn into_heap_secret_key(self) -> HeapSecretKey {
        HeapSecretKey(self.0.to_vec())
    }

    /// Returns the length of the key
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        N
    }

    /// Generates a new secret key, returning success if key created or
    /// failing if the desired key length is not between 1 and `isize::MAX`
    pub fn generate() -> Result<Self, SecretKeyError> {
        // Limitation described in https://github.com/orion-rs/orion/issues/130
        if N < 1 || N > (isize::MAX as usize) {
            return Err(SecretKeyError);
        }

        let mut key = [0; N];
        OsRng.fill_bytes(&mut key);

        Ok(Self(key))
    }

    /// Creates the key from the given byte slice, returning success if key created
    /// or failing if the byte slice does not match the desired key length
    pub fn from_slice(slice: &[u8]) -> Result<Self, SecretKeyError> {
        if slice.len() != N {
            return Err(SecretKeyError);
        }

        let mut value = [0u8; N];
        value[..N].copy_from_slice(slice);

        Ok(Self(value))
    }
}

impl<const N: usize> From<[u8; N]> for SecretKey<N> {
    fn from(arr: [u8; N]) -> Self {
        Self(arr)
    }
}

impl<const N: usize> FromStr for SecretKey<N> {
    type Err = SecretKeyError;

    /// Parse a str of hex as an N-byte secret key
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s).map_err(|_| SecretKeyError)?;
        Self::from_slice(&bytes)
    }
}

impl<const N: usize> fmt::Display for SecretKey<N> {
    /// Display an N-byte secret key as a hex string
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.unprotected_as_bytes()))
    }
}

/// Represents a secret key used with transport encryption and authentication that is stored on the
/// heap
#[derive(Clone, PartialEq, Eq)]
pub struct HeapSecretKey(Vec<u8>);

impl fmt::Debug for HeapSecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("HeapSecretKey")
            .field(&"**OMITTED**".to_string())
            .finish()
    }
}

impl HeapSecretKey {
    /// Returns byte slice to the key's bytes
    pub fn unprotected_as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the secret key and returns the key's bytes
    pub fn unprotected_into_bytes(self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// Returns the length of the key
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Generates a random key of `n` bytes in length.
    ///
    /// ### Note
    ///
    /// Will return an error if `n` < 1 or `n` > `isize::MAX`.
    pub fn generate(n: usize) -> Result<Self, SecretKeyError> {
        // Limitation described in https://github.com/orion-rs/orion/issues/130
        if n < 1 || n > (isize::MAX as usize) {
            return Err(SecretKeyError);
        }

        let mut key = Vec::new();
        let mut buf = [0; 32];

        // Continually generate a chunk of bytes and extend our key until we've reached
        // the appropriate length
        while key.len() < n {
            OsRng.fill_bytes(&mut buf);
            key.extend_from_slice(&buf[..std::cmp::min(n - key.len(), 32)]);
        }

        Ok(Self(key))
    }
}

impl From<Vec<u8>> for HeapSecretKey {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl<const N: usize> From<[u8; N]> for HeapSecretKey {
    fn from(arr: [u8; N]) -> Self {
        Self::from(arr.to_vec())
    }
}

impl<const N: usize> From<SecretKey<N>> for HeapSecretKey {
    fn from(key: SecretKey<N>) -> Self {
        key.into_heap_secret_key()
    }
}

impl FromStr for HeapSecretKey {
    type Err = SecretKeyError;

    /// Parse a str of hex as secret key on heap
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(hex::decode(s).map_err(|_| SecretKeyError)?))
    }
}

impl fmt::Display for HeapSecretKey {
    /// Display an N-byte secret key as a hex string
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.unprotected_as_bytes()))
    }
}

impl<const N: usize> PartialEq<[u8; N]> for HeapSecretKey {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.0.eq(other)
    }
}

impl<const N: usize> PartialEq<HeapSecretKey> for [u8; N] {
    fn eq(&self, other: &HeapSecretKey) -> bool {
        other.eq(self)
    }
}

impl<const N: usize> PartialEq<HeapSecretKey> for &[u8; N] {
    fn eq(&self, other: &HeapSecretKey) -> bool {
        other.eq(*self)
    }
}

impl PartialEq<[u8]> for HeapSecretKey {
    fn eq(&self, other: &[u8]) -> bool {
        self.0.eq(other)
    }
}

impl PartialEq<HeapSecretKey> for [u8] {
    fn eq(&self, other: &HeapSecretKey) -> bool {
        other.eq(self)
    }
}

impl PartialEq<HeapSecretKey> for &[u8] {
    fn eq(&self, other: &HeapSecretKey) -> bool {
        other.eq(*self)
    }
}

impl PartialEq<String> for HeapSecretKey {
    fn eq(&self, other: &String) -> bool {
        self.0.eq(other.as_bytes())
    }
}

impl PartialEq<HeapSecretKey> for String {
    fn eq(&self, other: &HeapSecretKey) -> bool {
        other.eq(self)
    }
}

impl PartialEq<HeapSecretKey> for &String {
    fn eq(&self, other: &HeapSecretKey) -> bool {
        other.eq(*self)
    }
}

impl PartialEq<str> for HeapSecretKey {
    fn eq(&self, other: &str) -> bool {
        self.0.eq(other.as_bytes())
    }
}

impl PartialEq<HeapSecretKey> for str {
    fn eq(&self, other: &HeapSecretKey) -> bool {
        other.eq(self)
    }
}

impl PartialEq<HeapSecretKey> for &str {
    fn eq(&self, other: &HeapSecretKey) -> bool {
        other.eq(*self)
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn secret_key_should_be_able_to_be_generated() {
        SecretKey::<0>::generate().unwrap_err();

        let key = SecretKey::<1>::generate().unwrap();
        assert_eq!(key.len(), 1);

        // NOTE: We aren't going to validate generating isize::MAX or +1 of that size because it
        //       takes a lot of time to do so
        let key = SecretKey::<100>::generate().unwrap();
        assert_eq!(key.len(), 100);
    }

    #[test]
    fn heap_secret_key_should_be_able_to_be_generated() {
        HeapSecretKey::generate(0).unwrap_err();

        let key = HeapSecretKey::generate(1).unwrap();
        assert_eq!(key.len(), 1);

        // NOTE: We aren't going to validate generating isize::MAX or +1 of that size because it
        //       takes a lot of time to do so
        let key = HeapSecretKey::generate(100).unwrap();
        assert_eq!(key.len(), 100);
    }

    // --- SecretKey: from_slice ---

    #[test]
    fn secret_key_from_slice_should_succeed_with_correct_length() {
        let data = [1u8, 2, 3, 4];
        let key = SecretKey::<4>::from_slice(&data).unwrap();
        assert_eq!(key.unprotected_as_bytes(), &data);
    }

    #[test]
    fn secret_key_from_slice_should_fail_with_wrong_length() {
        let data = [1u8, 2, 3];
        let result = SecretKey::<4>::from_slice(&data);
        assert!(result.is_err());
    }

    #[test]
    fn secret_key_from_slice_should_fail_with_empty_slice_for_nonzero_key() {
        let data: &[u8] = &[];
        let result = SecretKey::<4>::from_slice(data);
        assert!(result.is_err());
    }

    #[test]
    fn secret_key_from_slice_should_fail_with_longer_slice() {
        let data = [1u8, 2, 3, 4, 5];
        let result = SecretKey::<4>::from_slice(&data);
        assert!(result.is_err());
    }

    // --- SecretKey: FromStr / Display round-trip ---

    #[test]
    fn secret_key_display_should_produce_hex_string() {
        let key = SecretKey::<4>::from([0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(key.to_string(), "deadbeef");
    }

    #[test]
    fn secret_key_from_str_should_parse_hex_string() {
        let key: SecretKey<4> = "deadbeef".parse().unwrap();
        assert_eq!(key.unprotected_as_bytes(), &[0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn secret_key_from_str_and_display_should_round_trip() {
        let original = SecretKey::<16>::generate().unwrap();
        let hex_str = original.to_string();
        let recovered: SecretKey<16> = hex_str.parse().unwrap();
        assert_eq!(
            original.unprotected_as_bytes(),
            recovered.unprotected_as_bytes()
        );
    }

    #[test]
    fn secret_key_from_str_should_fail_on_invalid_hex() {
        let result = "not_valid_hex!".parse::<SecretKey<4>>();
        assert!(result.is_err());
    }

    #[test]
    fn secret_key_from_str_should_fail_on_wrong_length_hex() {
        // "aabb" is 2 bytes, but we need 4
        let result = "aabb".parse::<SecretKey<4>>();
        assert!(result.is_err());
    }

    // --- SecretKey: unprotected accessors ---

    #[test]
    fn secret_key_unprotected_as_bytes_should_return_slice() {
        let key = SecretKey::<3>::from([10, 20, 30]);
        let bytes = key.unprotected_as_bytes();
        assert_eq!(bytes, &[10, 20, 30]);
        assert_eq!(bytes.len(), 3);
    }

    #[test]
    fn secret_key_unprotected_as_byte_array_should_return_array_ref() {
        let key = SecretKey::<3>::from([10, 20, 30]);
        let arr: &[u8; 3] = key.unprotected_as_byte_array();
        assert_eq!(arr, &[10, 20, 30]);
    }

    #[test]
    fn secret_key_unprotected_into_byte_array_should_consume_and_return_array() {
        let key = SecretKey::<3>::from([10, 20, 30]);
        let arr: [u8; 3] = key.unprotected_into_byte_array();
        assert_eq!(arr, [10, 20, 30]);
    }

    // --- SecretKey: into_heap_secret_key ---

    #[test]
    fn secret_key_into_heap_secret_key_should_preserve_bytes() {
        let key = SecretKey::<4>::from([1, 2, 3, 4]);
        let heap_key = key.into_heap_secret_key();
        assert_eq!(heap_key.unprotected_as_bytes(), &[1, 2, 3, 4]);
        assert_eq!(heap_key.len(), 4);
    }

    // --- SecretKey: From<[u8; N]> ---

    #[test]
    fn secret_key_from_array_should_create_key_with_given_bytes() {
        let arr = [0xAAu8, 0xBB, 0xCC, 0xDD];
        let key: SecretKey<4> = SecretKey::from(arr);
        assert_eq!(key.unprotected_as_bytes(), &arr);
    }

    // --- SecretKey: Debug ---

    #[test]
    fn secret_key_debug_should_omit_key_content() {
        let key = SecretKey::<4>::from([1, 2, 3, 4]);
        let debug_str = format!("{:?}", key);
        assert!(
            debug_str.contains("OMITTED"),
            "Debug output should contain OMITTED but was: {}",
            debug_str
        );
        assert!(
            !debug_str.contains("01"),
            "Debug output should not contain key bytes"
        );
    }

    // --- SecretKeyError ---

    #[test]
    fn secret_key_error_should_convert_to_io_error() {
        let io_err: std::io::Error = SecretKeyError.into();
        assert_eq!(io_err.kind(), std::io::ErrorKind::InvalidData);
        assert!(io_err.to_string().contains("not valid secret key format"));
    }

    // --- HeapSecretKey: From<Vec<u8>> ---

    #[test]
    fn heap_secret_key_from_vec_should_preserve_bytes() {
        let v = vec![10u8, 20, 30, 40];
        let key = HeapSecretKey::from(v.clone());
        assert_eq!(key.unprotected_as_bytes(), &v[..]);
        assert_eq!(key.len(), 4);
    }

    // --- HeapSecretKey: From<[u8; N]> ---

    #[test]
    fn heap_secret_key_from_array_should_preserve_bytes() {
        let arr = [5u8, 10, 15, 20, 25];
        let key = HeapSecretKey::from(arr);
        assert_eq!(key.unprotected_as_bytes(), &arr);
        assert_eq!(key.len(), 5);
    }

    // --- HeapSecretKey: From<SecretKey<N>> ---

    #[test]
    fn heap_secret_key_from_secret_key_should_preserve_bytes() {
        let sk = SecretKey::<4>::from([0xAA, 0xBB, 0xCC, 0xDD]);
        let hk: HeapSecretKey = HeapSecretKey::from(sk);
        assert_eq!(hk.unprotected_as_bytes(), &[0xAA, 0xBB, 0xCC, 0xDD]);
    }

    // --- HeapSecretKey: FromStr / Display round-trip ---

    #[test]
    fn heap_secret_key_display_should_produce_hex_string() {
        let key = HeapSecretKey::from(vec![0xca, 0xfe, 0xba, 0xbe]);
        assert_eq!(key.to_string(), "cafebabe");
    }

    #[test]
    fn heap_secret_key_from_str_should_parse_hex_string() {
        let key: HeapSecretKey = "cafebabe".parse().unwrap();
        assert_eq!(key.unprotected_as_bytes(), &[0xca, 0xfe, 0xba, 0xbe]);
    }

    #[test]
    fn heap_secret_key_from_str_and_display_should_round_trip() {
        let original = HeapSecretKey::generate(32).unwrap();
        let hex_str = original.to_string();
        let recovered: HeapSecretKey = hex_str.parse().unwrap();
        assert_eq!(
            original.unprotected_as_bytes(),
            recovered.unprotected_as_bytes()
        );
    }

    #[test]
    fn heap_secret_key_from_str_should_fail_on_invalid_hex() {
        let result = "xyz_not_hex!!".parse::<HeapSecretKey>();
        assert!(result.is_err());
    }

    // --- HeapSecretKey: unprotected accessors ---

    #[test]
    fn heap_secret_key_unprotected_as_bytes_should_return_slice() {
        let key = HeapSecretKey::from(vec![1, 2, 3]);
        assert_eq!(key.unprotected_as_bytes(), &[1, 2, 3]);
    }

    #[test]
    fn heap_secret_key_unprotected_into_bytes_should_consume_and_return_vec() {
        let key = HeapSecretKey::from(vec![7, 8, 9]);
        let bytes = key.unprotected_into_bytes();
        assert_eq!(bytes, vec![7, 8, 9]);
    }

    // --- HeapSecretKey: Debug ---

    #[test]
    fn heap_secret_key_debug_should_omit_key_content() {
        let key = HeapSecretKey::from(vec![0xFF, 0xFE, 0xFD]);
        let debug_str = format!("{:?}", key);
        assert!(
            debug_str.contains("OMITTED"),
            "Debug output should contain OMITTED but was: {}",
            debug_str
        );
        assert!(
            !debug_str.contains("ff") && !debug_str.contains("FF"),
            "Debug output should not contain key bytes"
        );
    }

    // --- HeapSecretKey: PartialEq impls ---

    #[test]
    fn heap_secret_key_should_eq_byte_array() {
        let key = HeapSecretKey::from(vec![1, 2, 3, 4]);
        let arr: [u8; 4] = [1, 2, 3, 4];
        assert!(key == arr);
        assert!(arr == key);
    }

    #[test]
    fn heap_secret_key_should_eq_byte_array_ref() {
        let key = HeapSecretKey::from(vec![1, 2, 3, 4]);
        let arr: &[u8; 4] = &[1, 2, 3, 4];
        assert!(arr == &key);
    }

    #[test]
    fn heap_secret_key_should_eq_byte_slice() {
        let key = HeapSecretKey::from(vec![5, 6, 7]);
        let slice: &[u8] = &[5, 6, 7];
        assert!(key == *slice);
        assert!(*slice == key);
        assert!(slice == &key);
    }

    #[test]
    fn heap_secret_key_should_not_eq_different_byte_slice() {
        let key = HeapSecretKey::from(vec![5, 6, 7]);
        let slice: &[u8] = &[5, 6, 8];
        assert!(key != *slice);
    }

    #[test]
    fn heap_secret_key_should_eq_string() {
        // HeapSecretKey compares raw bytes to string bytes
        let raw_bytes = b"hello".to_vec();
        let key = HeapSecretKey::from(raw_bytes);
        let s = String::from("hello");
        assert!(key == s);
        assert!(s == key);
        assert!(&s == &key);
    }

    #[test]
    fn heap_secret_key_should_not_eq_different_string() {
        let key = HeapSecretKey::from(b"hello".to_vec());
        let s = String::from("world");
        assert!(key != s);
    }

    #[test]
    fn heap_secret_key_should_eq_str() {
        let raw_bytes = b"test".to_vec();
        let key = HeapSecretKey::from(raw_bytes);
        assert!(key == *"test");
        assert!(*"test" == key);
        assert!("test" == &key);
    }

    #[test]
    fn heap_secret_key_should_not_eq_different_str() {
        let key = HeapSecretKey::from(b"test".to_vec());
        assert!(key != *"other");
    }
}
