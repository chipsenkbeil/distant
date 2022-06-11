use derive_more::{Display, Error};
use rand::{rngs::OsRng, RngCore};
use std::{fmt, str::FromStr};

#[derive(Debug, Display, Error)]
pub struct SecretKeyError;

/// Represents a 32-byte secret key
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

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s).map_err(|_| SecretKeyError)?;
        Self::from_slice(&bytes)
    }
}

pub trait UnprotectedToHexKey {
    fn unprotected_to_hex_key(&self) -> String;
}

impl<const N: usize> UnprotectedToHexKey for SecretKey<N> {
    fn unprotected_to_hex_key(&self) -> String {
        hex::encode(self.unprotected_as_bytes())
    }
}
