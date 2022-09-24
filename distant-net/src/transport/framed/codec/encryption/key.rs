use derive_more::{Display, Error};
use rand::{rngs::OsRng, RngCore};
use std::{fmt, str::FromStr};

#[derive(Debug, Display, Error)]
pub struct SecretKeyError;

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